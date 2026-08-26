//! Compression utilities ported from `org.apache.lucene.util.compress`.
//!
//! This module provides LZ4 block compression/decompression and a specialized
//! packer for lowercase ASCII strings, matching the behavior of Apache Lucene
//! Core 10.5.0.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::store::{DataInput, DataOutput};

// -----------------------------------------------------------------------------
// LZ4
// -----------------------------------------------------------------------------

/// Maximum distance of a reference in the LZ4 window.
///
/// Equivalent to `org.apache.lucene.util.compress.LZ4.MAX_DISTANCE`.
pub const LZ4_MAX_DISTANCE: usize = 1 << 16;

const LZ4_MEMORY_USAGE: usize = 14;
const LZ4_MIN_MATCH: usize = 4;
const LZ4_LAST_LITERALS: usize = 5;
const LZ4_HASH_LOG_HC: u32 = 15;
const LZ4_HASH_TABLE_SIZE_HC: usize = 1 << LZ4_HASH_LOG_HC;
const LZ4_MAX_ATTEMPTS: usize = 256;

const HASH_MULTIPLIER: u32 = 0x9E3779B1;

fn lz4_hash(i: u32, hash_bits: u32) -> usize {
    let i = i.wrapping_mul(HASH_MULTIPLIER);
    (i >> (32 - hash_bits)) as usize
}

fn lz4_hash_hc(i: u32) -> usize {
    lz4_hash(i, LZ4_HASH_LOG_HC)
}

/// Reads four bytes as a `u32` using the native byte order of the Lucene
/// reference platform (x86_64 little-endian).
///
/// The Java original uses `BitUtil.VH_NATIVE_INT`, which is little-endian on the
/// hardware that produces the reference index files. Using little-endian here
/// ensures byte-for-byte compatibility with those files.
fn read_native_int(buf: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]])
}

/// Returns the number of consecutive matching bytes starting at `o1` and `o2`,
/// bounded by `limit`.
///
/// Mirrors `org.apache.lucene.util.compress.LZ4.commonBytes`, which is built
/// on `Arrays.mismatch`.
fn common_bytes(b: &[u8], o1: usize, o2: usize, limit: usize) -> usize {
    debug_assert!(o1 < o2);
    // The second range [o2, limit) is always the shorter (or equal) one because
    // o1 < o2. We count matching bytes up to the point where that range ends,
    // mirroring Java's Arrays.mismatch(b, o1, limit, b, o2, limit).
    let len = limit - o2;
    for i in 0..len {
        if b[o1 + i] != b[o2 + i] {
            return i;
        }
    }
    len
}

fn encode_len(l: usize, out: &mut dyn DataOutput) -> Result<()> {
    let mut l = l;
    while l >= 0xFF {
        out.write_byte(0xFF)?;
        l -= 0xFF;
    }
    out.write_byte(l as u8)
}

fn encode_literals(
    bytes: &[u8],
    token: u8,
    anchor: usize,
    literal_len: usize,
    out: &mut dyn DataOutput,
) -> Result<()> {
    out.write_byte(token)?;
    if literal_len >= 0x0F {
        encode_len(literal_len - 0x0F, out)?;
    }
    out.write_bytes(bytes, anchor, literal_len)
}

fn encode_last_literals(
    bytes: &[u8],
    anchor: usize,
    literal_len: usize,
    out: &mut dyn DataOutput,
) -> Result<()> {
    let token = (literal_len.min(0x0F) << 4) as u8;
    encode_literals(bytes, token, anchor, literal_len, out)
}

fn encode_sequence(
    bytes: &[u8],
    anchor: usize,
    match_ref: usize,
    match_off: usize,
    match_len: usize,
    out: &mut dyn DataOutput,
) -> Result<()> {
    let literal_len = match_off - anchor;
    debug_assert!(match_len >= LZ4_MIN_MATCH);
    let token =
        ((literal_len.min(0x0F) << 4) as u8) | ((match_len - LZ4_MIN_MATCH).min(0x0F) as u8);
    encode_literals(bytes, token, anchor, literal_len, out)?;

    let match_dec = match_off - match_ref;
    debug_assert!(match_dec > 0 && match_dec < (1 << 16));
    out.write_short(match_dec as i16)?;

    if match_len >= LZ4_MIN_MATCH + 0x0F {
        encode_len(match_len - 0x0F - LZ4_MIN_MATCH, out)?;
    }
    Ok(())
}

/// Hash table used by the LZ4 compressor to find repeated 4-byte sequences.
///
/// Equivalent to `org.apache.lucene.util.compress.LZ4.HashTable`.
pub trait Lz4HashTable: Send + Sync {
    /// Reset this hash table in order to compress the given content.
    fn reset(&mut self, bytes: &[u8], off: usize, len: usize) -> Result<()>;

    /// Initialize `dict_len` bytes to be used as a dictionary.
    fn init_dictionary(&mut self, bytes: &[u8], dict_len: usize);

    /// Advance the cursor to `off` and return an index that stores the same
    /// 4 bytes as `bytes[off..off+4]`. This may only be called on strictly
    /// increasing sequences of offsets. A return value of `None` indicates
    /// that no other index could be found.
    fn get(&mut self, bytes: &[u8], off: usize) -> Option<usize>;

    /// Return an index that is less than `off` and stores the same 4 bytes.
    /// Unlike [`Self::get`], this does not need to be called on increasing
    /// offsets. A return value of `None` indicates that no other index could
    /// be found.
    fn previous(&mut self, bytes: &[u8], off: usize) -> Option<usize>;
}

enum OffsetTable {
    U16(Vec<u16>),
    U32(Vec<u32>),
}

impl OffsetTable {
    fn size(&self) -> usize {
        match self {
            OffsetTable::U16(v) => v.len(),
            OffsetTable::U32(v) => v.len(),
        }
    }

    fn bits_per_value(&self) -> usize {
        match self {
            OffsetTable::U16(_) => 16,
            OffsetTable::U32(_) => 32,
        }
    }

    fn set(&mut self, index: usize, value: usize) {
        match self {
            OffsetTable::U16(v) => v[index] = value as u16,
            OffsetTable::U32(v) => v[index] = value as u32,
        }
    }

    fn get_and_set(&mut self, index: usize, value: usize) -> usize {
        match self {
            OffsetTable::U16(v) => {
                let prev = v[index] as usize;
                v[index] = value as u16;
                prev
            }
            OffsetTable::U32(v) => {
                let prev = v[index] as usize;
                v[index] = value as u32;
                prev
            }
        }
    }
}

/// Simple lossy hash table that only stores the last occurrence for each hash
/// on 2^14 bytes of memory.
///
/// Equivalent to `org.apache.lucene.util.compress.LZ4.FastCompressionHashTable`.
pub struct FastCompressionHashTable {
    base: usize,
    end: usize,
    // Lucene uses -1 as the sentinel before the first offset; isize preserves it.
    last_off: isize,
    hash_log: u32,
    table: OffsetTable,
}

impl Default for FastCompressionHashTable {
    fn default() -> Self {
        Self::new()
    }
}

impl FastCompressionHashTable {
    /// Creates a new empty fast compression hash table.
    pub fn new() -> Self {
        Self {
            base: 0,
            end: 0,
            last_off: 0,
            hash_log: 0,
            table: OffsetTable::U16(Vec::new()),
        }
    }
}

impl Lz4HashTable for FastCompressionHashTable {
    fn reset(&mut self, bytes: &[u8], off: usize, len: usize) -> Result<()> {
        if off
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?
            > bytes.len()
        {
            return Err(LuceneError::IllegalArgument(
                "offset or length out of bounds".to_string(),
            ));
        }
        let end = off + len;
        self.base = off;
        self.end = end;
        let bits_per_offset = if len.saturating_sub(LZ4_LAST_LITERALS) < (1 << 16) {
            16
        } else {
            32
        };
        let bits_per_offset_log = 32u32 - ((bits_per_offset - 1) as u32).leading_zeros();
        self.hash_log = LZ4_MEMORY_USAGE as u32 + 3 - bits_per_offset_log;
        let table_size = 1usize << self.hash_log;
        if self.table.size() < table_size || self.table.bits_per_value() < bits_per_offset {
            self.table = if bits_per_offset > 16 {
                OffsetTable::U32(vec![0; table_size])
            } else {
                OffsetTable::U16(vec![0; table_size])
            };
        }
        self.last_off = off as isize - 1;
        Ok(())
    }

    fn init_dictionary(&mut self, bytes: &[u8], dict_len: usize) {
        for i in 0..dict_len {
            let v = read_native_int(bytes, self.base + i);
            let h = lz4_hash(v, self.hash_log);
            self.table.set(h, i);
        }
        self.last_off += dict_len as isize;
    }

    fn get(&mut self, bytes: &[u8], off: usize) -> Option<usize> {
        debug_assert!(off as isize > self.last_off);
        debug_assert!(off < self.end);
        let v = read_native_int(bytes, off);
        let h = lz4_hash(v, self.hash_log);
        let ref_offset = self.base + self.table.get_and_set(h, off - self.base);
        self.last_off = off as isize;
        if ref_offset < off
            && off - ref_offset < LZ4_MAX_DISTANCE
            && read_native_int(bytes, ref_offset) == v
        {
            Some(ref_offset)
        } else {
            None
        }
    }

    fn previous(&mut self, _bytes: &[u8], _off: usize) -> Option<usize> {
        None
    }
}

/// Higher-precision hash table that stores up to 256 occurrences of 4-byte
/// sequences in the last 2^16 bytes.
///
/// Equivalent to `org.apache.lucene.util.compress.LZ4.HighCompressionHashTable`.
pub struct HighCompressionHashTable {
    base: usize,
    next: usize,
    end: usize,
    hash_table: Vec<i32>,
    chain_table: Vec<u16>,
    attempts: usize,
}

impl Default for HighCompressionHashTable {
    fn default() -> Self {
        Self::new()
    }
}

impl HighCompressionHashTable {
    /// Creates a new empty high-compression hash table.
    pub fn new() -> Self {
        Self {
            base: 0,
            next: 0,
            end: 0,
            hash_table: vec![-1; LZ4_HASH_TABLE_SIZE_HC],
            chain_table: vec![0xFFFF; LZ4_MAX_DISTANCE],
            attempts: 0,
        }
    }

    fn add_hash(&mut self, bytes: &[u8], off: usize) {
        let v = read_native_int(bytes, off);
        let h = lz4_hash_hc(v);
        // Java is `int delta = off - hashTable[h]; if (delta <= 0 || delta >=
        // MAX_DISTANCE) { delta = MAX_DISTANCE - 1; }` (`LZ4.java:475-484`),
        // and the subtraction is **signed** for two reasons. The unset slot
        // holds `-1`, and — the one that bites — `reset` deliberately keeps the
        // table when the previous span was shorter than the window
        // (`LZ4.java:411-431`), so a stale entry can hold an offset *larger*
        // than `off`. Computing this in `usize`, as this port used to, made
        // that case underflow: a debug build aborted and a release build
        // wrapped. The single guard below covers both, exactly as Java's does.
        let delta = off as i64 - i64::from(self.hash_table[h]);
        let delta = if delta <= 0 || delta >= LZ4_MAX_DISTANCE as i64 {
            LZ4_MAX_DISTANCE - 1
        } else {
            delta as usize
        };
        self.chain_table[off & (LZ4_MAX_DISTANCE - 1)] = delta as u16;
        self.hash_table[h] = off as i32;
    }
}

impl Lz4HashTable for HighCompressionHashTable {
    fn reset(&mut self, bytes: &[u8], off: usize, len: usize) -> Result<()> {
        if off
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?
            > bytes.len()
        {
            return Err(LuceneError::IllegalArgument(
                "offset or length out of bounds".to_string(),
            ));
        }
        let end = off + len;
        if self.end.saturating_sub(self.base) < self.chain_table.len() {
            let start_offset = self.base & (LZ4_MAX_DISTANCE - 1);
            let end_offset = if self.end == 0 {
                0
            } else {
                ((self.end - 1) & (LZ4_MAX_DISTANCE - 1)) + 1
            };
            if start_offset < end_offset {
                self.chain_table[start_offset..end_offset].fill(0xFFFF);
            } else {
                self.chain_table[0..end_offset].fill(0xFFFF);
                self.chain_table[start_offset..].fill(0xFFFF);
            }
        } else {
            self.hash_table.fill(-1);
            self.chain_table.fill(0xFFFF);
        }
        self.base = off;
        self.next = off;
        self.end = end;
        Ok(())
    }

    fn init_dictionary(&mut self, bytes: &[u8], dict_len: usize) {
        debug_assert_eq!(self.next, self.base);
        for i in 0..dict_len {
            self.add_hash(bytes, self.base + i);
        }
        self.next += dict_len;
    }

    fn get(&mut self, bytes: &[u8], off: usize) -> Option<usize> {
        debug_assert!(off >= self.next);
        debug_assert!(off < self.end);
        while self.next < off {
            self.add_hash(bytes, self.next);
            self.next += 1;
        }
        let v = read_native_int(bytes, off);
        let h = lz4_hash_hc(v);
        self.attempts = 0;

        let ref_i32 = self.hash_table[h];
        if ref_i32 < 0 || (ref_i32 as usize) >= off {
            return None;
        }
        let mut ref_offset = ref_i32 as usize;
        let min = self.base.max(off.saturating_sub(LZ4_MAX_DISTANCE - 1));
        while ref_offset >= min && self.attempts < LZ4_MAX_ATTEMPTS {
            if read_native_int(bytes, ref_offset) == v {
                return Some(ref_offset);
            }
            let delta = self.chain_table[ref_offset & (LZ4_MAX_DISTANCE - 1)] as usize;
            if delta == 0 || delta >= LZ4_MAX_DISTANCE {
                return None;
            }
            let next_ref = (ref_offset as isize).checked_sub(delta as isize)?;
            if next_ref < min as isize {
                return None;
            }
            ref_offset = next_ref as usize;
            self.attempts += 1;
        }
        None
    }

    fn previous(&mut self, bytes: &[u8], off: usize) -> Option<usize> {
        let v = read_native_int(bytes, off);
        let delta = self.chain_table[off & (LZ4_MAX_DISTANCE - 1)] as usize;
        if delta == 0 || delta >= LZ4_MAX_DISTANCE {
            return None;
        }
        let next_ref = (off as isize).checked_sub(delta as isize)?;
        if next_ref < self.base as isize {
            return None;
        }
        let mut ref_offset = next_ref as usize;
        while self.attempts < LZ4_MAX_ATTEMPTS {
            if read_native_int(bytes, ref_offset) == v {
                return Some(ref_offset);
            }
            let delta = self.chain_table[ref_offset & (LZ4_MAX_DISTANCE - 1)] as usize;
            if delta == 0 || delta >= LZ4_MAX_DISTANCE {
                return None;
            }
            let next_ref = (ref_offset as isize).checked_sub(delta as isize)?;
            if next_ref < self.base as isize {
                return None;
            }
            ref_offset = next_ref as usize;
            self.attempts += 1;
        }
        None
    }
}

/// LZ4 compression and decompression routines.
///
/// Equivalent to `org.apache.lucene.util.compress.LZ4`.
pub struct Lz4;

impl Lz4 {
    /// Compress `bytes[off:off+len]` into `out` using at most 16kB of memory.
    ///
    /// `ht` should not be shared across threads but can be reused.
    ///
    /// Equivalent to `org.apache.lucene.util.compress.LZ4.compress`.
    pub fn compress(
        bytes: &[u8],
        off: usize,
        len: usize,
        out: &mut dyn DataOutput,
        ht: &mut dyn Lz4HashTable,
    ) -> Result<()> {
        Self::compress_with_dictionary(bytes, off, 0, len, out, ht)
    }

    /// Compress `bytes[dict_off+dict_len:dict_off+dict_len+len]` into `out`,
    /// using `bytes[dict_off:dict_off+dict_len]` as a dictionary. `dict_len`
    /// must not exceed [`LZ4_MAX_DISTANCE`].
    ///
    /// Equivalent to `org.apache.lucene.util.compress.LZ4.compressWithDictionary`.
    pub fn compress_with_dictionary(
        bytes: &[u8],
        dict_off: usize,
        dict_len: usize,
        len: usize,
        out: &mut dyn DataOutput,
        ht: &mut dyn Lz4HashTable,
    ) -> Result<()> {
        if dict_off.checked_add(dict_len).ok_or_else(|| {
            LuceneError::IllegalArgument("dictOff + dictLen overflowed".to_string())
        })? > bytes.len()
        {
            return Err(LuceneError::IllegalArgument(
                "dictOff or dictLen out of bounds".to_string(),
            ));
        }
        if dict_off
            .checked_add(dict_len)
            .and_then(|x| x.checked_add(len))
            .ok_or_else(|| {
                LuceneError::IllegalArgument("dictOff + dictLen + len overflowed".to_string())
            })?
            > bytes.len()
        {
            return Err(LuceneError::IllegalArgument(
                "dictOff + dictLen + len out of bounds".to_string(),
            ));
        }
        if dict_len > LZ4_MAX_DISTANCE {
            return Err(LuceneError::IllegalArgument(format!(
                "dictLen must not be greater than 64kB, but got {dict_len}"
            )));
        }

        let end = dict_off + dict_len + len;
        let mut off = dict_off + dict_len;
        let mut anchor = off;

        if len > LZ4_LAST_LITERALS + LZ4_MIN_MATCH {
            let limit = end - LZ4_LAST_LITERALS;
            let match_limit = limit - LZ4_MIN_MATCH;
            ht.reset(bytes, dict_off, dict_len + len)?;
            ht.init_dictionary(bytes, dict_len);

            loop {
                // Find a match.
                let ref_offset = loop {
                    if off >= match_limit {
                        break None;
                    }
                    if let Some(r) = ht.get(bytes, off) {
                        debug_assert!(r >= dict_off && r < off);
                        debug_assert_eq!(read_native_int(bytes, r), read_native_int(bytes, off));
                        break Some(r);
                    }
                    off += 1;
                };
                let Some(mut ref_offset) = ref_offset else {
                    break;
                };

                // Compute match length.
                let mut match_len = LZ4_MIN_MATCH
                    + common_bytes(
                        bytes,
                        ref_offset + LZ4_MIN_MATCH,
                        off + LZ4_MIN_MATCH,
                        limit,
                    );

                // Try to find a better (longer) match.
                let min = off.saturating_sub(LZ4_MAX_DISTANCE - 1).max(dict_off);
                let mut r = ht.previous(bytes, ref_offset);
                while let Some(prev_ref) = r {
                    if prev_ref < min {
                        break;
                    }
                    debug_assert_eq!(
                        read_native_int(bytes, prev_ref),
                        read_native_int(bytes, off)
                    );
                    let prev_match_len = LZ4_MIN_MATCH
                        + common_bytes(bytes, prev_ref + LZ4_MIN_MATCH, off + LZ4_MIN_MATCH, limit);
                    if prev_match_len > match_len {
                        ref_offset = prev_ref;
                        match_len = prev_match_len;
                    }
                    r = ht.previous(bytes, prev_ref);
                }

                encode_sequence(bytes, anchor, ref_offset, off, match_len, out)?;
                off += match_len;
                anchor = off;

                if off > limit {
                    break;
                }
            }
        }

        let literal_len = end - anchor;
        debug_assert!(literal_len >= LZ4_LAST_LITERALS || literal_len == len);
        encode_last_literals(bytes, anchor, literal_len, out)
    }

    /// Decompress at least `decompressed_len` bytes into `dest[d_off:]`. The
    /// destination buffer must be large enough to hold all decompressed data.
    ///
    /// Returns the final destination offset.
    ///
    /// Equivalent to `org.apache.lucene.util.compress.LZ4.decompress`.
    pub fn decompress(
        input: &mut dyn DataInput,
        decompressed_len: usize,
        dest: &mut [u8],
        d_off: usize,
    ) -> Result<usize> {
        let dest_end = d_off + decompressed_len;
        if dest_end > dest.len() {
            return Err(LuceneError::IllegalArgument(
                "destination buffer too small".to_string(),
            ));
        }
        let mut d_off = d_off;

        loop {
            // Literals.
            let token = input.read_byte()? as usize;
            let mut literal_len = token >> 4;
            if literal_len == 0x0F {
                loop {
                    let len = input.read_byte()?;
                    literal_len += len as usize;
                    if len != 0xFF {
                        break;
                    }
                }
            }
            if literal_len != 0 {
                // A corrupt stream can claim more literals than the buffer can
                // hold. Java raises `ArrayIndexOutOfBoundsException` here; this
                // port reports corruption rather than panicking, because the
                // bytes come straight off disk and are not trusted. The bound
                // is the whole buffer, not `dest_end`: decoding only a prefix
                // of a block legitimately writes a few bytes past it, which is
                // why callers pass a padded destination.
                if literal_len > dest.len() - d_off {
                    return Err(LuceneError::CorruptIndex(format!(
                        "LZ4 literal run of {literal_len} bytes overruns the buffer \
                         ({} bytes left)",
                        dest.len() - d_off
                    )));
                }
                input.read_bytes(dest, d_off, literal_len)?;
                d_off += literal_len;
            }

            if d_off >= dest_end {
                break;
            }

            // Match.
            let match_dec = input.read_short()? as u16 as usize;
            if match_dec == 0 {
                return Err(LuceneError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "offset 0 is invalid",
                )));
            }

            let mut match_len = token & 0x0F;
            if match_len == 0x0F {
                loop {
                    let len = input.read_byte()?;
                    match_len += len as usize;
                    if len != 0xFF {
                        break;
                    }
                }
            }
            match_len += LZ4_MIN_MATCH;

            // Both copies below index `d_off - match_dec`, and the match may
            // not reach behind the start of the block.
            if match_dec > d_off {
                return Err(LuceneError::CorruptIndex(format!(
                    "LZ4 match reaches {match_dec} bytes back from offset {d_off}"
                )));
            }
            if match_len > dest.len() - d_off {
                return Err(LuceneError::CorruptIndex(format!(
                    "LZ4 match of {match_len} bytes overruns the buffer ({} bytes left)",
                    dest.len() - d_off
                )));
            }

            let fast_len = (match_len + 7) & !7;
            if match_dec < match_len || d_off + fast_len > dest_end {
                // Overlap -> naive incremental copy.
                let mut ref_pos = d_off - match_dec;
                let match_end = d_off + match_len;
                while d_off < match_end {
                    dest[d_off] = dest[ref_pos];
                    ref_pos += 1;
                    d_off += 1;
                }
            } else {
                // No overlap -> bulk copy.
                let src_start = d_off - match_dec;
                dest.copy_within(src_start..src_start + fast_len, d_off);
                d_off += match_len;
            }

            if d_off >= dest_end {
                break;
            }
        }

        Ok(d_off)
    }
}

// -----------------------------------------------------------------------------
// LowercaseAsciiCompression
// -----------------------------------------------------------------------------

/// Compact encoding for strings that mostly contain lowercase ASCII characters,
/// digits, '.', '-' and '_'.
///
/// Equivalent to `org.apache.lucene.util.compress.LowercaseAsciiCompression`.
pub struct LowercaseAsciiCompression;

impl LowercaseAsciiCompression {
    fn is_compressible(b: u8) -> bool {
        let high3_bits = (b as usize + 1) & !0x1F;
        high3_bits == 0x20 || high3_bits == 0x60
    }

    /// Compress `input[0..len]` into `out`. Returns `false` if the content
    /// cannot be compressed. The number of bytes written is guaranteed to be
    /// less than `len` when compression succeeds.
    ///
    /// `tmp` must have length at least `len` and is used as scratch space.
    ///
    /// Equivalent to `org.apache.lucene.util.compress.LowercaseAsciiCompression.compress`.
    pub fn compress(
        input: &[u8],
        len: usize,
        tmp: &mut [u8],
        out: &mut dyn DataOutput,
    ) -> Result<bool> {
        if len < 8 {
            return Ok(false);
        }
        if len > tmp.len() {
            return Err(LuceneError::IllegalArgument(
                "tmp buffer is smaller than len".to_string(),
            ));
        }

        // Count exceptions and fail compression if there are too many of them.
        let max_exceptions = len >> 5;
        let mut previous_exception_index = 0usize;
        let mut num_exceptions = 0usize;
        for (i, &b) in input.iter().enumerate().take(len) {
            if !Self::is_compressible(b) {
                while i - previous_exception_index > 0xFF {
                    num_exceptions += 1;
                    previous_exception_index += 0xFF;
                }
                num_exceptions += 1;
                if num_exceptions > max_exceptions {
                    return Ok(false);
                }
                previous_exception_index = i;
            }
        }
        debug_assert!(num_exceptions <= max_exceptions);

        // Move all bytes to the [0, 0x40) range (6 bits).
        let compressed_len = len - (len >> 2);
        debug_assert!(compressed_len < len);
        for (i, &b) in input.iter().enumerate().take(len) {
            let b = b as usize + 1;
            tmp[i] = ((b & 0x1F) | ((b & 0x40) >> 1)) as u8;
        }

        // Pack the bytes so that 4 ASCII chars occupy 3 bytes.
        let mut o = 0usize;
        #[allow(clippy::needless_range_loop)]
        for i in compressed_len..len {
            tmp[o] |= (tmp[i] & 0x30) << 2;
            o += 1;
        }
        #[allow(clippy::needless_range_loop)]
        for i in compressed_len..len {
            tmp[o] |= (tmp[i] & 0x0C) << 4;
            o += 1;
        }
        #[allow(clippy::needless_range_loop)]
        for i in compressed_len..len {
            tmp[o] |= (tmp[i] & 0x03) << 6;
            o += 1;
        }
        debug_assert!(o <= compressed_len);

        out.write_bytes(tmp, 0, compressed_len)?;

        // Record exceptions.
        out.write_v_int(num_exceptions as i32)?;
        if num_exceptions > 0 {
            previous_exception_index = 0;
            let mut num_exceptions2 = 0usize;
            for i in 0..len {
                let b = input[i];
                if !Self::is_compressible(b) {
                    while i - previous_exception_index > 0xFF {
                        out.write_byte(0xFF)?;
                        previous_exception_index += 0xFF;
                        out.write_byte(input[previous_exception_index])?;
                        num_exceptions2 += 1;
                    }
                    out.write_byte((i - previous_exception_index) as u8)?;
                    previous_exception_index = i;
                    out.write_byte(b)?;
                    num_exceptions2 += 1;
                }
            }
            if num_exceptions != num_exceptions2 {
                return Err(LuceneError::IllegalState(format!(
                    "{num_exceptions} <> {num_exceptions2}"
                )));
            }
        }

        Ok(true)
    }

    /// Decompress data that has been compressed with [`Self::compress`].
    /// `len` must be the original length, not the compressed length.
    ///
    /// Equivalent to `org.apache.lucene.util.compress.LowercaseAsciiCompression.decompress`.
    pub fn decompress(input: &mut dyn DataInput, out: &mut [u8], len: usize) -> Result<()> {
        if len > out.len() {
            return Err(LuceneError::IllegalArgument(
                "output buffer too small".to_string(),
            ));
        }
        let saved = len >> 2;
        let compressed_len = len - saved;

        // Copy the packed bytes.
        input.read_bytes(out, 0, compressed_len)?;

        // Restore the leading 2 bits of each packed byte into whole bytes.
        #[allow(clippy::needless_range_loop)]
        for i in 0..saved {
            out[compressed_len + i] = ((out[i] & 0xC0) >> 2)
                | ((out[saved + i] & 0xC0) >> 4)
                | ((out[(saved << 1) + i] & 0xC0) >> 6);
        }

        // Move back to the original range.
        for byte in out.iter_mut().take(len) {
            let b = *byte as usize;
            *byte = (((b & 0x1F) | 0x20 | ((b & 0x20) << 1)) - 1) as u8;
        }

        // Restore exceptions.
        let num_exceptions = input.read_v_int()? as usize;
        let mut i = 0usize;
        for _ in 0..num_exceptions {
            i += input.read_byte()? as usize;
            out[i] = input.read_byte()?;
        }
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ByteArrayDataInput, ByteArrayDataOutput};

    fn lz4_round_trip(input: &[u8], ht: &mut dyn Lz4HashTable) {
        let mut out = ByteArrayDataOutput::new();
        Lz4::compress(input, 0, input.len(), &mut out, ht).unwrap();
        let compressed = out.into_inner();

        let mut decompressed = vec![0u8; input.len()];
        let mut input_stream = ByteArrayDataInput::from_slice(&compressed);
        Lz4::decompress(&mut input_stream, input.len(), &mut decompressed, 0).unwrap();
        assert_eq!(
            decompressed,
            input,
            "LZ4 round-trip failed for {} bytes",
            input.len()
        );
    }

    fn lower_round_trip(input: &[u8]) {
        let mut tmp = vec![0u8; input.len()];
        let mut out = ByteArrayDataOutput::new();
        let ok =
            LowercaseAsciiCompression::compress(input, input.len(), &mut tmp, &mut out).unwrap();
        if !ok {
            return;
        }
        let compressed = out.into_inner();
        assert!(
            compressed.len() < input.len(),
            "compression must shrink the input"
        );

        let mut decompressed = vec![0u8; input.len()];
        let mut input_stream = ByteArrayDataInput::from_slice(&compressed);
        LowercaseAsciiCompression::decompress(&mut input_stream, &mut decompressed, input.len())
            .unwrap();
        assert_eq!(decompressed, input);
    }

    // -------------------------------------------------------------------------
    // LZ4 unit tests
    // -------------------------------------------------------------------------

    #[test]
    fn lz4_fast_empty() {
        let input = b"";
        let mut ht = FastCompressionHashTable::new();
        lz4_round_trip(input, &mut ht);
    }

    #[test]
    fn lz4_fast_tiny() {
        let mut ht = FastCompressionHashTable::new();
        lz4_round_trip(b"a", &mut ht);
        lz4_round_trip(b"abc", &mut ht);
        lz4_round_trip(b"abcdefghij", &mut ht);
    }

    #[test]
    fn lz4_fast_repeated_run() {
        let mut ht = FastCompressionHashTable::new();
        lz4_round_trip(&[b'a'; 10], &mut ht);
        lz4_round_trip(&[b'a'; 100], &mut ht);
        lz4_round_trip(&vec![b'a'; 1000], &mut ht);
    }

    #[test]
    fn lz4_fast_repeated_pattern() {
        let mut ht = FastCompressionHashTable::new();
        lz4_round_trip(b"abcabcabcabc", &mut ht);
        let s = "the quick brown fox ".repeat(50);
        lz4_round_trip(s.as_bytes(), &mut ht);
    }

    #[test]
    fn lz4_fast_random_ascii() {
        let mut ht = FastCompressionHashTable::new();
        let s = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.";
        lz4_round_trip(s.as_bytes(), &mut ht);
    }

    #[test]
    fn lz4_high_various_inputs() {
        let mut ht = HighCompressionHashTable::new();
        lz4_round_trip(b"", &mut ht);
        lz4_round_trip(b"hello world", &mut ht);
        lz4_round_trip(&vec![b'x'; 500], &mut ht);
        let s = "abcabcabcabcabcabcabcabc".repeat(20);
        lz4_round_trip(s.as_bytes(), &mut ht);
    }

    #[test]
    fn lz4_fast_known_vector_aaaaaaaaaa() {
        // Java's FastCompressionHashTable emits the input unchanged for this
        // length: matchLimit = (10 - LAST_LITERALS) - MIN_MATCH = 1, so the
        // search loop terminates before it can evaluate offset 1.
        let input = vec![b'a'; 10];
        let expected = &[
            0xA0u8, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61, 0x61,
        ];
        let mut out = ByteArrayDataOutput::new();
        let mut ht = FastCompressionHashTable::new();
        Lz4::compress(&input, 0, input.len(), &mut out, &mut ht).unwrap();
        let compressed = out.into_inner();
        assert_eq!(&compressed, expected);

        let mut decompressed = vec![0u8; input.len()];
        let mut input_stream = ByteArrayDataInput::from_slice(&compressed);
        Lz4::decompress(&mut input_stream, input.len(), &mut decompressed, 0).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn lz4_fast_known_vector_abcabcabcabc() {
        // Java's FastCompressionHashTable again emits the input unchanged:
        // matchLimit = (12 - LAST_LITERALS) - MIN_MATCH = 3, so the search loop
        // terminates at offset 3 before it can find the repeated "abca" block.
        let input = b"abcabcabcabc";
        let expected = &[
            0xC0u8, 0x61, 0x62, 0x63, 0x61, 0x62, 0x63, 0x61, 0x62, 0x63, 0x61, 0x62, 0x63,
        ];
        let mut out = ByteArrayDataOutput::new();
        let mut ht = FastCompressionHashTable::new();
        Lz4::compress(input, 0, input.len(), &mut out, &mut ht).unwrap();
        let compressed = out.into_inner();
        assert_eq!(&compressed, expected);

        let mut decompressed = vec![0u8; input.len()];
        let mut input_stream = ByteArrayDataInput::from_slice(&compressed);
        Lz4::decompress(&mut input_stream, input.len(), &mut decompressed, 0).unwrap();
        assert_eq!(&decompressed, input);
    }

    #[test]
    fn lz4_decompress_rejects_zero_offset() {
        // Token 0x00 (0 literals, 0 match length with no extra), short 0x0000.
        let data = &[0x00u8, 0x00, 0x00];
        let mut dest = vec![0u8; 4];
        let mut input = ByteArrayDataInput::from_slice(data);
        let result = Lz4::decompress(&mut input, 4, &mut dest, 0);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("offset 0"));
    }

    #[test]
    fn lz4_decompress_overlap_run() {
        // Hand-crafted stream that exercises the overlap copy path in the
        // decompressor: token 0x10 (1 literal, match len 4), literal 'a',
        // matchDec 1, then last 4 literals. This is *not* the output that the
        // fast compressor produces for 10 'a's (which is all literals).
        let data = &[0x11u8, 0x61, 0x01, 0x00, 0x40, 0x61, 0x61, 0x61, 0x61];
        let mut dest = vec![0u8; 10];
        let mut input = ByteArrayDataInput::from_slice(data);
        Lz4::decompress(&mut input, 10, &mut dest, 0).unwrap();
        assert_eq!(dest, vec![b'a'; 10]);
    }

    #[test]
    fn lz4_compress_bounds_checks() {
        let input = b"hello";
        let mut out = ByteArrayDataOutput::new();
        let mut ht = FastCompressionHashTable::new();
        assert!(Lz4::compress(input, 0, 10, &mut out, &mut ht).is_err());
        assert!(Lz4::compress(input, 10, 1, &mut out, &mut ht).is_err());
    }

    // -------------------------------------------------------------------------
    // LowercaseAsciiCompression unit tests
    // -------------------------------------------------------------------------

    #[test]
    fn lower_empty_and_short() {
        let mut tmp = vec![0u8; 8];
        let mut out = ByteArrayDataOutput::new();
        assert!(!LowercaseAsciiCompression::compress(b"", 0, &mut tmp, &mut out).unwrap());
        assert!(!LowercaseAsciiCompression::compress(b"hello", 5, &mut tmp, &mut out).unwrap());
    }

    #[test]
    fn lower_round_trip_compressible() {
        lower_round_trip(b"hello.world-1_2");
        lower_round_trip(b"the_quick_brown_fox_jumps_over_the_lazy_dog");
        lower_round_trip(b"abcdefghijklmnopqrstuvwxyz");
        lower_round_trip(b"1234567890.-_");
        lower_round_trip("abc".repeat(20).as_bytes());
        lower_round_trip(b"a b c d e f g"); // spaces are compressible
    }

    #[test]
    fn lower_rejects_too_many_exceptions() {
        let input = b"hello World"; // uppercase 'W' is the only exception, len=11 -> max=0
        let mut tmp = vec![0u8; input.len()];
        let mut out = ByteArrayDataOutput::new();
        assert!(
            !LowercaseAsciiCompression::compress(input, input.len(), &mut tmp, &mut out).unwrap()
        );
    }

    #[test]
    fn lower_known_vector_helloworld() {
        // Hand-traced from the Java path for "hello.world-1_2" (15 bytes,
        // all compressible, no exceptions).
        let input = b"hello.world-1_2";
        let expected = &[
            0x69u8, 0xA6, 0x6D, 0x2D, 0x30, 0x0F, 0xB8, 0x30, 0xF3, 0x2D, 0x25, 0x0E, 0x00,
        ];
        let mut tmp = vec![0u8; input.len()];
        let mut out = ByteArrayDataOutput::new();
        assert!(
            LowercaseAsciiCompression::compress(input, input.len(), &mut tmp, &mut out).unwrap()
        );
        let compressed = out.into_inner();
        assert_eq!(&compressed, expected);

        let mut decompressed = vec![0u8; input.len()];
        let mut input_stream = ByteArrayDataInput::from_slice(&compressed);
        LowercaseAsciiCompression::decompress(&mut input_stream, &mut decompressed, input.len())
            .unwrap();
        assert_eq!(&decompressed, input);
    }

    #[test]
    fn lower_with_exception_round_trip() {
        // 32 characters, one exception -> max_exceptions = 1.
        let input = [b'a'; 31];
        let mut input = input.to_vec();
        input.push(b'W');
        lower_round_trip(&input);
    }

    #[test]
    fn lower_decompress_bounds_checks() {
        let data = &[0x00u8, 0x00, 0x00]; // 3 bytes of packed data + 0 exceptions
        let mut out = vec![0u8; 4];
        let mut input = ByteArrayDataInput::from_slice(data);
        // len=4 needs compressed_len=3 and saved=1, but input is only 3 bytes.
        assert!(LowercaseAsciiCompression::decompress(&mut input, &mut out, 4).is_err());
    }
    /// A decoder must never panic on bytes it did not write.
    ///
    /// Java's `LZ4.decompress` raises `ArrayIndexOutOfBoundsException` for a
    /// truncated or hostile stream; this port returns
    /// `LuceneError::CorruptIndex`. Before the bounds checks these inputs
    /// aborted the process in a debug build ("attempt to subtract with
    /// overflow") and produced a wild slice index in a release build.
    #[test]
    fn a_corrupt_lz4_stream_is_reported_and_never_panics() {
        // token 0x00: no literals, then a match at distance 1 with nothing
        // decoded yet, so the match reaches behind the start of the block.
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("match before the block start", vec![0x00, 0x01, 0x00]),
            ("huge match distance", vec![0x00, 0xFF, 0xFF]),
            (
                "literal run longer than the buffer",
                vec![
                    0xF0, 0xFF, 0x01, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
                ],
            ),
            (
                "match longer than the buffer",
                vec![0x1F, b'a', 0x01, 0x00, 0xFF, 0xFF, 0xFF],
            ),
        ];
        for (name, bytes) in cases {
            let mut input = ByteArrayDataInput::new(bytes);
            let mut dest = vec![0u8; 8];
            let outcome = Lz4::decompress(&mut input, 8, &mut dest, 0);
            assert!(
                outcome.is_err(),
                "{name}: a corrupt stream must be reported, got {outcome:?}"
            );
        }
    }

    /// The bounds checks must not reject anything a real encoder produces.
    #[test]
    fn the_corruption_checks_do_not_reject_valid_streams() {
        for length in [0usize, 1, 7, 64, 1000, 40_000] {
            let data: Vec<u8> = (0..length).map(|i| ((i * 13) % 61) as u8).collect();
            let mut out = ByteArrayDataOutput::new();
            let mut table = FastCompressionHashTable::new();
            Lz4::compress(&data, 0, data.len(), &mut out, &mut table).unwrap();
            let mut input = ByteArrayDataInput::new(out.into_inner());
            let mut dest = vec![0u8; data.len() + 7];
            let produced = Lz4::decompress(&mut input, data.len(), &mut dest, 0).unwrap();
            assert_eq!(produced, data.len());
            assert_eq!(&dest[..data.len()], &data[..]);
        }
    }
    /// Regression test: a reused high-compression table can hold a stale offset
    /// larger than the current one.
    ///
    /// `HighCompressionHashTable::reset` keeps the hash table whenever the
    /// previous span was shorter than the 64 KiB window (`LZ4.java:411-431`),
    /// so `hashTable[h]` can point *ahead* of the offset being added when the
    /// next span starts over at a lower offset. Java computes
    /// `off - hashTable[h]` in a signed `int` and guards `delta <= 0`; doing it
    /// in `usize` underflowed — a debug build aborted with "attempt to subtract
    /// with overflow", a release build wrapped.
    ///
    /// The construction is deliberate: the first span is long enough for the
    /// match loop to hash a high offset, carries a marker well inside the
    /// region it hashes, and is still short enough for `reset` to keep the
    /// table; the second span opens with the same marker at offset 0. That is
    /// the only shape in which the stale entry lies ahead of the new offset.
    #[test]
    fn a_reused_high_compression_table_survives_a_stale_forward_offset() {
        const MARKER: &[u8; 4] = b"ZZZZ";

        let mut first: Vec<u8> = (0..1000u32).map(|i| ((i * 37 + 11) % 251) as u8).collect();
        first[500..504].copy_from_slice(MARKER);
        let mut second: Vec<u8> = MARKER.to_vec();
        second.extend((0..200u32).map(|i| ((i * 53 + 7) % 251) as u8));

        let mut table = HighCompressionHashTable::new();
        let mut out = ByteArrayDataOutput::new();
        Lz4::compress(&first, 0, first.len(), &mut out, &mut table).unwrap();
        // The marker's hash slot now holds offset 500; this span hashes it
        // again at offset 0.
        Lz4::compress(&second, 0, second.len(), &mut out, &mut table).unwrap();

        // And everything written must still decode to what went in.
        let mut input = ByteArrayDataInput::new(out.into_inner());
        for expected in [&first[..], &second[..]] {
            let mut dest = vec![0u8; expected.len() + 7];
            let produced = Lz4::decompress(&mut input, expected.len(), &mut dest, 0).unwrap();
            assert_eq!(produced, expected.len());
            assert_eq!(&dest[..expected.len()], expected);
        }
    }

    /// The high-compression table must agree with Java's formula for an unset
    /// slot too, which Java leaves to the same guard rather than special-casing.
    #[test]
    fn the_high_compression_table_round_trips_from_a_fresh_state() {
        for length in [4usize, 64, 4096, 70_000] {
            let data: Vec<u8> = (0..length).map(|i| ((i / 7) % 251) as u8).collect();
            let mut table = HighCompressionHashTable::new();
            let mut out = ByteArrayDataOutput::new();
            Lz4::compress(&data, 0, data.len(), &mut out, &mut table).unwrap();
            let mut input = ByteArrayDataInput::new(out.into_inner());
            let mut dest = vec![0u8; data.len() + 7];
            Lz4::decompress(&mut input, data.len(), &mut dest, 0).unwrap();
            assert_eq!(&dest[..data.len()], &data[..], "length {length}");
        }
    }
}
