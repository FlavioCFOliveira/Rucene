// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! Reproductions of the JDK 25 behaviours the ICU4J classes of this crate
//! rely on: `java.nio.ByteBuffer` (the read-only heap buffers ICU reads its
//! data through), the class-path resource lookup of `Class` and
//! `ClassLoader` over the embedded resources, `java.io.DataOutputStream`,
//! the `java.lang.Character` surrogate arithmetic, and the unchecked
//! exceptions the JDK throws where ICU relies on them.

use std::sync::Arc;

use rucene::error::{LuceneError, LuceneErrorKind, Result};
pub use rucene::store::java_nio_file::{InputStream, OutputStream};

use crate::class_path_data;

/// `java.util.Currency`.
pub mod currency;
/// `java.math`: `BigInteger`, `BigDecimal`, `MathContext`, `RoundingMode`.
pub mod math;
/// `java.text`: `Format`, `Format.Field`, `FieldPosition`, `AttributedString`.
pub mod text;

// ---------------------------------------------------------------------------
// Exceptions
// ---------------------------------------------------------------------------

/// `new <class>()` for an unchecked JDK exception without a message.
pub(crate) fn unchecked_no_message(class: &'static str) -> LuceneError {
    LuceneError::from(LuceneErrorKind::Unchecked {
        class,
        message: None,
        cause: None,
    })
}

/// Throws `new IndexOutOfBoundsException()` (`message` `None`) or
/// `new IndexOutOfBoundsException(message)`: a panic with the Java text (see
/// the crate documentation on the `IndexOutOfBoundsException` family).
pub fn index_out_of_bounds(message: Option<String>) -> ! {
    match message {
        Some(m) => panic!("java.lang.IndexOutOfBoundsException: {m}"),
        None => panic!("java.lang.IndexOutOfBoundsException"),
    }
}

/// Throws `new StringIndexOutOfBoundsException(int index)`: the message
/// `String index out of range: <index>` (JDK 25), as a panic.
pub fn string_index_out_of_bounds(index: i32) -> ! {
    panic!("java.lang.StringIndexOutOfBoundsException: String index out of range: {index}")
}

/// Throws `new ArrayIndexOutOfBoundsException(int index)`: the message
/// `Array index out of range: <index>` (JDK 25), as a panic.
pub fn array_index_out_of_bounds(index: i32) -> ! {
    panic!("java.lang.ArrayIndexOutOfBoundsException: Array index out of range: {index}")
}

/// `new IllegalArgumentException()`: no message.
pub fn illegal_argument_no_message() -> LuceneError {
    unchecked_no_message("java.lang.IllegalArgumentException")
}

/// `new ArithmeticException(message)`.
pub fn arithmetic(message: &str) -> LuceneError {
    LuceneError::from(LuceneErrorKind::Arithmetic(message.to_string()))
}

/// `new IllegalStateException(message)`.
pub fn illegal_state(message: impl Into<String>) -> LuceneError {
    LuceneError::illegal_state(message)
}

/// `new NullPointerException(message)`.
pub fn null_pointer(message: &str) -> LuceneError {
    LuceneError::null_pointer_with_message(message)
}

/// The message of `Preconditions.outOfBoundsCheckIndex`, which the JDK's
/// index checks (`Objects.checkIndex`, the absolute `ByteBuffer` getters)
/// throw `IndexOutOfBoundsException` with: `Index <i> out of bounds for
/// length <n>`.
///
/// # Panics
///
/// With that message when `index` is outside `0..length`.
fn check_index(index: i32, length: i32) -> usize {
    if index < 0 || index >= length {
        panic!("java.lang.IndexOutOfBoundsException: Index {index} out of bounds for length {length}");
    }
    index as usize
}

// ---------------------------------------------------------------------------
// java.lang.Character
// ---------------------------------------------------------------------------

/// `Character.MIN_SUPPLEMENTARY_CODE_POINT`.
pub const MIN_SUPPLEMENTARY_CODE_POINT: i32 = 0x10000;

/// `Character.toCodePoint(char high, char low)`.
pub fn to_code_point(high: u16, low: u16) -> i32 {
    ((high as i32) << 10)
        .wrapping_add(low as i32)
        .wrapping_add(MIN_SUPPLEMENTARY_CODE_POINT - (0xD800 << 10) - 0xDC00)
}

/// `Character.codePointAt(CharSequence seq, int index)`.
///
/// # Panics
///
/// Where Java throws `IndexOutOfBoundsException`: `index` outside the text.
pub fn code_point_at(seq: &[u16], index: i32) -> i32 {
    let c1 = seq[index as usize];
    if is_high_surrogate(c1) {
        let next = index as usize + 1;
        if next < seq.len() {
            let c2 = seq[next];
            if is_low_surrogate(c2) {
                return to_code_point(c1, c2);
            }
        }
    }
    c1 as i32
}

/// `Character.isHighSurrogate(char)`.
pub fn is_high_surrogate(ch: u16) -> bool {
    (0xD800..=0xDBFF).contains(&ch)
}

/// `Character.isLowSurrogate(char)`.
pub fn is_low_surrogate(ch: u16) -> bool {
    (0xDC00..=0xDFFF).contains(&ch)
}

/// `Character.charCount(int codePoint)`.
pub fn char_count(code_point: i32) -> i32 {
    if code_point >= MIN_SUPPLEMENTARY_CODE_POINT {
        2
    } else {
        1
    }
}

/// `StringBuilder.appendCodePoint(int)`: the code point as one or two units.
///
/// # Errors
///
/// `IllegalArgumentException` when `code_point` is not a valid code point
/// (`Character.toChars`: `Not a valid Unicode code point: 0x<HEX>`).
pub fn append_code_point(sb: &mut Vec<u16>, code_point: i32) -> Result<()> {
    if (0..MIN_SUPPLEMENTARY_CODE_POINT).contains(&code_point) {
        sb.push(code_point as u16);
    } else if (MIN_SUPPLEMENTARY_CODE_POINT..=0x10FFFF).contains(&code_point) {
        sb.push((((code_point as u32) >> 10) + (0xD800 - (0x10000 >> 10))) as u16);
        sb.push((((code_point as u32) & 0x3FF) + 0xDC00) as u16);
    } else {
        return Err(LuceneError::illegal_argument(format!(
            "Not a valid Unicode code point: 0x{:X}",
            code_point as u32
        )));
    }
    Ok(())
}

/// `Character.digit(char ch, int radix)`: the value of a Unicode decimal
/// digit (`Nd`) or of a Latin letter (ASCII or fullwidth, `A`/`a` = 10) in
/// `radix`, or -1 when `ch` has no value below `radix` or the radix is
/// outside `2..=36`.
pub fn character_digit(ch: i32, radix: i32) -> i32 {
    if !(2..=36).contains(&radix) {
        return -1;
    }
    use rucene_analysis_common::jdk::character as c;
    let value = if c::is_digit(ch) {
        c::get_numeric_value(ch)
    } else if (b'A' as i32..=b'Z' as i32).contains(&ch) {
        ch - b'A' as i32 + 10
    } else if (b'a' as i32..=b'z' as i32).contains(&ch) {
        ch - b'a' as i32 + 10
    } else if (0xFF21..=0xFF3A).contains(&ch) {
        ch - 0xFF21 + 10
    } else if (0xFF41..=0xFF5A).contains(&ch) {
        ch - 0xFF41 + 10
    } else {
        -1
    };
    if value >= 0 && value < radix {
        value
    } else {
        -1
    }
}

/// `Integer.parseInt(String s, int radix)` (JDK 25).
///
/// # Errors
///
/// `NumberFormatException` with the JDK's `For input string: "<s>"[ under
/// radix <r>]` message.
pub fn parse_int(s: &[u16], radix: i32) -> Result<i32> {
    let text = to_rust_string(s);
    let nfe = || LuceneError::number_format_for_input_string(&text, radix as u32);
    let len = s.len();
    if len == 0 {
        return Err(nfe());
    }
    let mut negative = false;
    let mut i = 0usize;
    let mut limit = -i32::MAX;
    let first_char = s[0];
    if first_char < b'0' as u16 {
        // Possible leading "+" or "-"
        if first_char == b'-' as u16 {
            negative = true;
            limit = i32::MIN;
        } else if first_char != b'+' as u16 {
            return Err(nfe());
        }
        if len == 1 {
            // Cannot have lone "+" or "-"
            return Err(nfe());
        }
        i += 1;
    }
    let multmin = limit / radix;
    let mut result = 0i32;
    while i < len {
        // Accumulating negatively avoids surprises near MAX_VALUE
        let digit = character_digit(s[i] as i32, radix);
        i += 1;
        if digit < 0 || result < multmin {
            return Err(nfe());
        }
        result *= radix;
        if result < limit + digit {
            return Err(nfe());
        }
        result -= digit;
    }
    Ok(if negative { result } else { -result })
}

/// `Character.isSpaceChar(char)`: the general category is `Zs`, `Zl` or
/// `Zp`.
pub fn is_space_char(ch: u16) -> bool {
    matches!(
        rucene_analysis_common::jdk::character::get_type(ch as i32),
        12..=14
    )
}

/// The UTF-16 units of a string literal, Java's `String` constant.
pub fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// `new String(char[])` as a Rust string, for the texts the port passes to
/// Rust APIs (messages, resource names); unpaired surrogates become U+FFFD.
pub fn to_rust_string(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

/// `String.indexOf(String str, int fromIndex)` over UTF-16 units.
pub fn index_of(source: &[u16], target: &[u16], from_index: i32) -> i32 {
    let from = from_index.max(0) as usize;
    let n = source.len();
    let m = target.len();
    if from >= n {
        return if m == 0 { n as i32 } else { -1 };
    }
    if m == 0 {
        return from as i32;
    }
    if m > n {
        return -1;
    }
    (from..=n - m)
        .find(|&i| &source[i..i + m] == target)
        .map_or(-1, |i| i as i32)
}

/// `String.lastIndexOf(String str, int fromIndex)` over UTF-16 units.
pub fn last_index_of(source: &[u16], target: &[u16], from_index: i32) -> i32 {
    let n = source.len() as i32;
    let m = target.len() as i32;
    let right_index = n - m;
    let mut from = from_index;
    if from > right_index {
        from = right_index;
    }
    if from < 0 {
        return -1;
    }
    let mut i = from;
    while i >= 0 {
        let s = i as usize;
        if source[s..s + m as usize] == *target {
            return i;
        }
        i -= 1;
    }
    -1
}

/// `String.indexOf(int ch, int fromIndex)` for a `char` value.
pub fn index_of_unit(source: &[u16], ch: u16, from_index: i32) -> i32 {
    let from = from_index.max(0) as usize;
    if from >= source.len() {
        return -1;
    }
    source[from..]
        .iter()
        .position(|&c| c == ch)
        .map_or(-1, |p| (from + p) as i32)
}

/// `String.lastIndexOf(int ch, int fromIndex)` for a `char` value.
pub fn last_index_of_unit(source: &[u16], ch: u16, from_index: i32) -> i32 {
    if from_index < 0 || source.is_empty() {
        return -1;
    }
    let mut i = (from_index as usize).min(source.len() - 1) as i32;
    while i >= 0 {
        if source[i as usize] == ch {
            return i;
        }
        i -= 1;
    }
    -1
}

// ---------------------------------------------------------------------------
// java.nio.ByteBuffer
// ---------------------------------------------------------------------------

/// `java.nio.ByteOrder`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    /// `ByteOrder.BIG_ENDIAN`, the order of a new buffer.
    BigEndian,
    /// `ByteOrder.LITTLE_ENDIAN`.
    LittleEndian,
}

/// The heap array a [`ByteBuffer`] views, shared by `duplicate()` and
/// `slice()`.
#[derive(Debug, Clone)]
struct Storage(Arc<[u8]>);

impl Storage {
    fn bytes(&self) -> &[u8] {
        &self.0
    }
}

/// A `java.nio.ByteBuffer` as ICU reads its data through it: a heap buffer
/// with a position, a limit, a capacity, a mark and a byte order over an
/// array that `duplicate()` and `slice()` share.
///
/// Equivalent to the subset of `java.nio.HeapByteBuffer` ICU4J calls. ICU
/// never writes into the buffers it reads its data from, so the port keeps
/// only the read operations; a clone is `duplicate()` with the byte order
/// kept (Java's `duplicate()` resets the order of the copy to big-endian,
/// which [`duplicate`](Self::duplicate) reproduces).
#[derive(Debug, Clone)]
pub struct ByteBuffer {
    storage: Storage,
    /// The array index of buffer index 0 (`slice()` offsets).
    offset: usize,
    mark: i32,
    position: i32,
    limit: i32,
    capacity: i32,
    order: ByteOrder,
}

impl ByteBuffer {
    /// `ByteBuffer.wrap(byte[] array)`.
    pub fn wrap(array: Vec<u8>) -> Self {
        let len = array.len() as i32;
        Self::wrap_range(array, 0, len)
    }

    /// `ByteBuffer.wrap(byte[] array, int offset, int length)`: capacity
    /// `array.length`, position `offset`, limit `offset + length`.
    ///
    /// # Panics
    ///
    /// Where Java throws `IndexOutOfBoundsException`: the range is outside
    /// the array.
    pub fn wrap_range(array: Vec<u8>, offset: i32, length: i32) -> Self {
        let cap = array.len() as i32;
        assert!(
            offset >= 0 && length >= 0 && offset <= cap - length,
            "java.lang.IndexOutOfBoundsException"
        );
        Self {
            storage: Storage(Arc::from(array)),
            offset: 0,
            mark: -1,
            position: offset,
            limit: offset + length,
            capacity: cap,
            order: ByteOrder::BigEndian,
        }
    }

    /// `capacity()`.
    pub fn capacity(&self) -> i32 {
        self.capacity
    }

    /// `position()`.
    pub fn position(&self) -> i32 {
        self.position
    }

    /// `position(int newPosition)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `new_position` is negative or above
    /// the limit, with the JDK's message.
    pub fn set_position(&mut self, new_position: i32) -> Result<&mut Self> {
        if new_position > self.limit || new_position < 0 {
            let msg = if new_position > self.limit {
                format!(
                    "newPosition > limit: ({} > {})",
                    new_position, self.limit
                )
            } else {
                format!("newPosition < 0: ({new_position} < 0)")
            };
            return Err(LuceneError::illegal_argument(msg));
        }
        if self.mark > new_position {
            self.mark = -1;
        }
        self.position = new_position;
        Ok(self)
    }

    /// `limit()`.
    pub fn limit(&self) -> i32 {
        self.limit
    }

    /// `limit(int newLimit)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `new_limit` is negative or above the
    /// capacity, with the JDK's message.
    pub fn set_limit(&mut self, new_limit: i32) -> Result<&mut Self> {
        if new_limit > self.capacity || new_limit < 0 {
            let msg = if new_limit > self.capacity {
                format!(
                    "newLimit > capacity: ({} > {})",
                    new_limit, self.capacity
                )
            } else {
                format!("newLimit < 0: ({new_limit} < 0)")
            };
            return Err(LuceneError::illegal_argument(msg));
        }
        self.limit = new_limit;
        if self.position > new_limit {
            self.position = new_limit;
        }
        if self.mark > new_limit {
            self.mark = -1;
        }
        Ok(self)
    }

    /// `mark()`.
    pub fn mark(&mut self) -> &mut Self {
        self.mark = self.position;
        self
    }

    /// `reset()`.
    ///
    /// # Errors
    ///
    /// `InvalidMarkException` when no mark is set.
    pub fn reset(&mut self) -> Result<&mut Self> {
        if self.mark < 0 {
            return Err(unchecked_no_message("java.nio.InvalidMarkException"));
        }
        self.position = self.mark;
        Ok(self)
    }

    /// `rewind()`.
    pub fn rewind(&mut self) -> &mut Self {
        self.position = 0;
        self.mark = -1;
        self
    }

    /// `remaining()`.
    pub fn remaining(&self) -> i32 {
        let rem = self.limit - self.position;
        rem.max(0)
    }

    /// `hasRemaining()`.
    pub fn has_remaining(&self) -> bool {
        self.position < self.limit
    }

    /// `order()`.
    pub fn order(&self) -> ByteOrder {
        self.order
    }

    /// `order(ByteOrder bo)`.
    pub fn set_order(&mut self, order: ByteOrder) -> &mut Self {
        self.order = order;
        self
    }

    /// `duplicate()`: a buffer sharing the bytes, with the same position,
    /// limit, capacity and mark, and the big-endian order of a new buffer.
    pub fn duplicate(&self) -> Self {
        let mut d = self.clone();
        d.order = ByteOrder::BigEndian;
        d
    }

    /// `slice()`: a big-endian buffer over the remaining bytes, position 0.
    pub fn slice(&self) -> Self {
        let pos = self.position;
        let lim = self.limit;
        let rem = if pos <= lim { lim - pos } else { 0 };
        Self {
            storage: self.storage.clone(),
            offset: self.offset + pos as usize,
            mark: -1,
            position: 0,
            limit: rem,
            capacity: rem,
            order: ByteOrder::BigEndian,
        }
    }

    fn byte_at(&self, index: usize) -> u8 {
        self.storage.bytes()[self.offset + index]
    }

    /// The index of a relative read of `n` bytes, advancing the position.
    fn next_get_index(&mut self, n: i32) -> Result<usize> {
        let p = self.position;
        if self.limit - p < n {
            return Err(unchecked_no_message("java.nio.BufferUnderflowException"));
        }
        self.position = p + n;
        Ok(p as usize)
    }

    /// The index of an absolute read of `n` bytes at `index`.
    fn check_absolute(&self, index: i32, n: i32) -> usize {
        check_index(index, self.limit - n + 1)
    }

    fn read_u16(&self, i: usize) -> u16 {
        let b = [self.byte_at(i), self.byte_at(i + 1)];
        match self.order {
            ByteOrder::BigEndian => u16::from_be_bytes(b),
            ByteOrder::LittleEndian => u16::from_le_bytes(b),
        }
    }

    fn read_u32(&self, i: usize) -> u32 {
        let b = [
            self.byte_at(i),
            self.byte_at(i + 1),
            self.byte_at(i + 2),
            self.byte_at(i + 3),
        ];
        match self.order {
            ByteOrder::BigEndian => u32::from_be_bytes(b),
            ByteOrder::LittleEndian => u32::from_le_bytes(b),
        }
    }

    fn read_u64(&self, i: usize) -> u64 {
        let mut b = [0u8; 8];
        for (k, v) in b.iter_mut().enumerate() {
            *v = self.byte_at(i + k);
        }
        match self.order {
            ByteOrder::BigEndian => u64::from_be_bytes(b),
            ByteOrder::LittleEndian => u64::from_le_bytes(b),
        }
    }

    /// `get()`.
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` at the limit.
    pub fn get(&mut self) -> Result<i8> {
        let i = self.next_get_index(1)?;
        Ok(self.byte_at(i) as i8)
    }

    /// `get(int index)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` outside `0..limit`.
    pub fn get_at(&self, index: i32) -> i8 {
        let i = self.check_absolute(index, 1);
        self.byte_at(i) as i8
    }

    /// `get(byte[] dst)` (and `get(dst, 0, dst.length)`).
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` when fewer than `dst.len()` bytes remain.
    pub fn get_bytes(&mut self, dst: &mut [u8]) -> Result<()> {
        let n = dst.len() as i32;
        let i = self.next_get_index(n)?;
        let start = self.offset + i;
        dst.copy_from_slice(&self.storage.bytes()[start..start + n as usize]);
        Ok(())
    }

    /// `getChar()`.
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` when fewer than two bytes remain.
    pub fn get_char(&mut self) -> Result<u16> {
        let i = self.next_get_index(2)?;
        Ok(self.read_u16(i))
    }

    /// `getChar(int index)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when `index..index + 2` is outside the
    /// limit.
    pub fn get_char_at(&self, index: i32) -> u16 {
        let i = self.check_absolute(index, 2);
        self.read_u16(i)
    }

    /// `getShort()`.
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` when fewer than two bytes remain.
    pub fn get_short(&mut self) -> Result<i16> {
        let i = self.next_get_index(2)?;
        Ok(self.read_u16(i) as i16)
    }

    /// `getShort(int index)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when `index..index + 2` is outside the
    /// limit.
    pub fn get_short_at(&self, index: i32) -> i16 {
        let i = self.check_absolute(index, 2);
        self.read_u16(i) as i16
    }

    /// `getInt()`.
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` when fewer than four bytes remain.
    pub fn get_int(&mut self) -> Result<i32> {
        let i = self.next_get_index(4)?;
        Ok(self.read_u32(i) as i32)
    }

    /// `getInt(int index)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when `index..index + 4` is outside the
    /// limit.
    pub fn get_int_at(&self, index: i32) -> i32 {
        let i = self.check_absolute(index, 4);
        self.read_u32(i) as i32
    }

    /// `getLong()`.
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` when fewer than eight bytes remain.
    pub fn get_long(&mut self) -> Result<i64> {
        let i = self.next_get_index(8)?;
        Ok(self.read_u64(i) as i64)
    }

    /// `asCharBuffer().get(char[] dst)`: `dst.len()` units from the
    /// position, in the buffer's order, without moving the position.
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` when the view has fewer units.
    pub fn view_chars(&self, dst: &mut [u16]) -> Result<()> {
        self.view_check(dst.len(), 2)?;
        let p = self.position as usize;
        for (k, v) in dst.iter_mut().enumerate() {
            *v = self.read_u16(p + 2 * k);
        }
        Ok(())
    }

    /// `asShortBuffer().get(short[] dst)`.
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` when the view has fewer values.
    pub fn view_shorts(&self, dst: &mut [i16]) -> Result<()> {
        self.view_check(dst.len(), 2)?;
        let p = self.position as usize;
        for (k, v) in dst.iter_mut().enumerate() {
            *v = self.read_u16(p + 2 * k) as i16;
        }
        Ok(())
    }

    /// `asIntBuffer().get(int[] dst)`.
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` when the view has fewer values.
    pub fn view_ints(&self, dst: &mut [i32]) -> Result<()> {
        self.view_check(dst.len(), 4)?;
        let p = self.position as usize;
        for (k, v) in dst.iter_mut().enumerate() {
            *v = self.read_u32(p + 4 * k) as i32;
        }
        Ok(())
    }

    /// `asLongBuffer().get(long[] dst)`.
    ///
    /// # Errors
    ///
    /// `BufferUnderflowException` when the view has fewer values.
    pub fn view_longs(&self, dst: &mut [i64]) -> Result<()> {
        self.view_check(dst.len(), 8)?;
        let p = self.position as usize;
        for (k, v) in dst.iter_mut().enumerate() {
            *v = self.read_u64(p + 8 * k) as i64;
        }
        Ok(())
    }

    /// The capacity check of a view's bulk `get`: the view spans
    /// `remaining() / size` values.
    fn view_check(&self, count: usize, size: usize) -> Result<()> {
        if count > self.remaining() as usize / size {
            return Err(unchecked_no_message("java.nio.BufferUnderflowException"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Class-path resources: Class, ClassLoader, InputStream
// ---------------------------------------------------------------------------

/// An `InputStream` over an embedded class-path resource, as
/// `ClassLoader.getResourceAsStream` opens it for a jar entry: `available()`
/// is the number of bytes not yet read, as the JDK's jar entry streams
/// answer.
#[derive(Debug)]
pub struct ResourceInputStream {
    name: &'static str,
    data: &'static [u8],
    pos: usize,
    mark: usize,
}

impl InputStream for ResourceInputStream {
    fn read_byte(&mut self) -> std::io::Result<i32> {
        if self.pos < self.data.len() {
            let b = self.data[self.pos];
            self.pos += 1;
            Ok(b as i32)
        } else {
            Ok(-1)
        }
    }

    fn read(&mut self, b: &mut [u8], off: usize, len: usize) -> std::io::Result<i32> {
        if len == 0 {
            return Ok(0);
        }
        let rem = self.data.len() - self.pos;
        if rem == 0 {
            return Ok(-1);
        }
        let n = len.min(rem);
        b[off..off + n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n as i32)
    }

    fn skip(&mut self, n: i64) -> std::io::Result<i64> {
        if n <= 0 {
            return Ok(0);
        }
        let k = (n as u64).min((self.data.len() - self.pos) as u64) as usize;
        self.pos += k;
        Ok(k as i64)
    }

    fn available(&mut self) -> std::io::Result<i32> {
        Ok((self.data.len() - self.pos).min(i32::MAX as usize) as i32)
    }

    fn close(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    fn mark(&mut self, _readlimit: i32) {
        self.mark = self.pos;
    }

    fn reset(&mut self) -> std::io::Result<()> {
        self.pos = self.mark;
        Ok(())
    }

    fn mark_supported(&self) -> bool {
        true
    }

    fn stream_to_string(&self) -> String {
        format!("ResourceInputStream[{}]", self.name)
    }
}

/// A `java.lang.ClassLoader` as ICU looks resources up through it.
///
/// **Divergence.** A JVM resolves a resource name against the class path of
/// a class loader at run time (`ClassLoader.getResourceAsStream`); this
/// crate has no class path, so the application class loader answers from
/// the resources embedded in the crate (`class_path_data`), byte-identical
/// to the `icu4j-77.1.jar` entries of the same names, and the bootstrap
/// class loader, which in Java sees only the JDK's own modules, answers no
/// ICU resource. `toString()` is the class name of the JDK's application
/// class loader without the `@<identity hash>` suffix, which has no
/// counterpart here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassLoader {
    /// The application (system) class loader, whose class path holds
    /// `icu4j-77.1.jar`.
    Application,
    /// A loader whose parent is the bootstrap loader
    /// (`ClassLoaderUtil.BootstrapClassLoader`).
    Bootstrap,
}

impl ClassLoader {
    /// `ClassLoader.getSystemClassLoader()`, which is also every thread's
    /// context class loader and the defining loader of the ICU classes.
    pub fn system() -> Self {
        ClassLoader::Application
    }

    /// `getResourceAsStream(String name)`: a stream over the resource
    /// `name` (absolute, no leading `/`), `None` when there is none.
    pub fn get_resource_as_stream(&self, name: &str) -> Option<ResourceInputStream> {
        self.find(name).map(|(name, data)| ResourceInputStream {
            name,
            data,
            pos: 0,
            mark: 0,
        })
    }

    /// `getResource(String name) != null`.
    pub fn has_resource(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    fn find(&self, name: &str) -> Option<(&'static str, &'static [u8])> {
        match self {
            ClassLoader::Application => class_path_data::RESOURCES
                .binary_search_by(|(n, _)| (*n).cmp(name))
                .ok()
                .map(|i| class_path_data::RESOURCES[i]),
            ClassLoader::Bootstrap => None,
        }
    }

    /// `toString()`.
    pub fn to_java_string(&self) -> String {
        match self {
            ClassLoader::Application => "jdk.internal.loader.ClassLoaders$AppClassLoader".into(),
            ClassLoader::Bootstrap => {
                "com.ibm.icu.impl.ClassLoaderUtil$BootstrapClassLoader".into()
            }
        }
    }
}

/// A `java.lang.Class` as ICU resolves resources relative to it: its binary
/// name, which gives its package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JavaClass {
    name: &'static str,
}

impl JavaClass {
    /// The class of binary name `name` (`com.ibm.icu.impl.ICUData`).
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }

    /// `getName()`.
    pub fn get_name(&self) -> &'static str {
        self.name
    }

    /// `getPackage().getName()`.
    pub fn get_package_name(&self) -> &'static str {
        match self.name.rfind('.') {
            Some(i) => &self.name[..i],
            None => "",
        }
    }

    /// `getClassLoader()`: the application class loader, which defines
    /// every ICU class.
    pub fn get_class_loader(&self) -> Option<ClassLoader> {
        Some(ClassLoader::Application)
    }

    /// `Class.resolveName`: an absolute name without its leading `/`, or a
    /// relative one prefixed with the package path.
    fn resolve_name(&self, name: &str) -> String {
        if let Some(stripped) = name.strip_prefix('/') {
            stripped.to_string()
        } else {
            let pkg = self.get_package_name();
            if pkg.is_empty() {
                name.to_string()
            } else {
                format!("{}/{}", pkg.replace('.', "/"), name)
            }
        }
    }

    /// `getResourceAsStream(String name)`.
    pub fn get_resource_as_stream(&self, name: &str) -> Option<ResourceInputStream> {
        ClassLoader::Application.get_resource_as_stream(&self.resolve_name(name))
    }

    /// `getResource(String name) != null`.
    pub fn has_resource(&self, name: &str) -> bool {
        ClassLoader::Application.has_resource(&self.resolve_name(name))
    }
}

// ---------------------------------------------------------------------------
// java.io.DataOutputStream
// ---------------------------------------------------------------------------

/// `java.io.DataOutputStream` over an [`OutputStream`]: big-endian
/// primitives and the count of bytes written (`size()`).
pub struct DataOutputStream<'a> {
    out: &'a mut dyn OutputStream,
    written: i32,
}

impl<'a> DataOutputStream<'a> {
    /// `new DataOutputStream(OutputStream out)`.
    pub fn new(out: &'a mut dyn OutputStream) -> Self {
        Self { out, written: 0 }
    }

    fn put(&mut self, bytes: &[u8]) -> Result<()> {
        self.out.write(bytes, 0, bytes.len())?;
        self.written = self.written.wrapping_add(bytes.len() as i32);
        if self.written < 0 {
            self.written = i32::MAX;
        }
        Ok(())
    }

    /// `writeByte(int v)`.
    ///
    /// # Errors
    ///
    /// The `IOException` of the stream.
    pub fn write_byte(&mut self, v: i32) -> Result<()> {
        self.out.write_byte(v)?;
        self.written = self.written.saturating_add(1);
        Ok(())
    }

    /// `writeChar(int v)`.
    ///
    /// # Errors
    ///
    /// The `IOException` of the stream.
    pub fn write_char(&mut self, v: i32) -> Result<()> {
        self.put(&(v as u16).to_be_bytes())
    }

    /// `writeShort(int v)`.
    ///
    /// # Errors
    ///
    /// The `IOException` of the stream.
    pub fn write_short(&mut self, v: i32) -> Result<()> {
        self.put(&(v as u16).to_be_bytes())
    }

    /// `writeInt(int v)`.
    ///
    /// # Errors
    ///
    /// The `IOException` of the stream.
    pub fn write_int(&mut self, v: i32) -> Result<()> {
        self.put(&v.to_be_bytes())
    }

    /// `writeLong(long v)`.
    ///
    /// # Errors
    ///
    /// The `IOException` of the stream.
    pub fn write_long(&mut self, v: i64) -> Result<()> {
        self.put(&v.to_be_bytes())
    }

    /// `size()`.
    pub fn size(&self) -> i32 {
        self.written
    }
}

// ---------------------------------------------------------------------------
// java.net.URL and ClassLoader.getResources over the embedded resources
// ---------------------------------------------------------------------------

/// The location the embedded resources stand for: the `icu4j-77.1.jar` of
/// the class path.
pub const EMBEDDED_JAR: &str = "file:/icu4j-77.1.jar";

/// A `java.net.URL` as ICU enumerates resources through it: its protocol,
/// its path and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaUrl {
    protocol: String,
    path: String,
    text: String,
}

impl JavaUrl {
    /// `new URL(String spec)` for the `file:` and `jar:` forms ICU builds:
    /// the protocol is the text before the first `:`, the path the rest
    /// (for `jar:`, the whole `<url>!/<entry>` remainder, as
    /// `URL.getPath()` returns it).
    ///
    /// # Errors
    ///
    /// `MalformedURLException("no protocol: <spec>")` when there is no
    /// protocol.
    pub fn new(spec: &str) -> Result<Self> {
        match spec.find(':') {
            Some(i) if i > 0 => Ok(Self {
                protocol: spec[..i].to_string(),
                path: spec[i + 1..].to_string(),
                text: spec.to_string(),
            }),
            _ => Err(LuceneError::from(std::io::Error::other(format!(
                "no protocol: {spec}"
            )))),
        }
    }

    /// `getProtocol()`.
    pub fn get_protocol(&self) -> &str {
        &self.protocol
    }

    /// `getPath()`.
    pub fn get_path(&self) -> &str {
        &self.path
    }

    /// The entries of the jar a `jar:` URL names, in the jar's order
    /// (`JarURLConnection.getJarFile().entries()`), as `(name,
    /// is_directory)`: the embedded resources and the directories that
    /// contain them.
    ///
    /// # Errors
    ///
    /// `IOException` when the URL is not a `jar:` URL of the embedded jar.
    pub fn jar_entries(&self) -> Result<Vec<(String, bool)>> {
        let jar = self
            .path
            .find("!/")
            .map(|i| &self.path[..i])
            .filter(|j| *j == EMBEDDED_JAR);
        if self.protocol != "jar" || jar.is_none() {
            return Err(LuceneError::from(std::io::Error::other(format!(
                "no jar file for {}",
                self.text
            ))));
        }
        let mut entries: Vec<(String, bool)> = Vec::new();
        let mut dirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for (name, _) in class_path_data::RESOURCES {
            let mut end = 0;
            while let Some(i) = name[end..].find('/') {
                end += i + 1;
                let dir = &name[..end];
                if dirs.insert(dir.to_string()) {
                    entries.push((dir.to_string(), true));
                }
            }
            entries.push((name.to_string(), false));
        }
        Ok(entries)
    }
}

/// `toString()`.
impl std::fmt::Display for JavaUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl ClassLoader {
    /// `getResources(String name)`: the URLs of the resource or directory
    /// `name` on the class path, here the `jar:` URL into the embedded jar
    /// when an embedded resource is `name` or lies below the directory
    /// `name` (a name ending with `/`); the bootstrap loader finds none.
    pub fn get_resources(&self, name: &str) -> Vec<JavaUrl> {
        match self {
            ClassLoader::Application => {
                let found = class_path_data::RESOURCES
                    .iter()
                    .any(|(n, _)| *n == name || (name.ends_with('/') && n.starts_with(name)));
                if found {
                    vec![JavaUrl::new(&format!("jar:{EMBEDDED_JAR}!/{name}"))
                        .expect("a jar URL has a protocol")]
                } else {
                    Vec::new()
                }
            }
            ClassLoader::Bootstrap => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// java.text.ParsePosition, java.util.BitSet, Character helpers
// ---------------------------------------------------------------------------

/// A parse position: the index parsing continues from and the index of a
/// parse error.
///
/// Equivalent to `java.text.ParsePosition` (`int` indexes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsePosition {
    index: i32,
    error_index: i32,
}

impl ParsePosition {
    /// `new ParsePosition(int index)`: the error index is -1.
    pub fn new(index: i32) -> Self {
        Self { index, error_index: -1 }
    }

    /// `getIndex()`.
    pub fn get_index(&self) -> i32 {
        self.index
    }

    /// `setIndex(int index)`.
    pub fn set_index(&mut self, index: i32) {
        self.index = index;
    }

    /// `getErrorIndex()`.
    pub fn get_error_index(&self) -> i32 {
        self.error_index
    }

    /// `setErrorIndex(int ei)`.
    pub fn set_error_index(&mut self, ei: i32) {
        self.error_index = ei;
    }

    /// `hashCode()`: `(errorIndex << 16) | index`.
    pub fn hash_code(&self) -> i32 {
        (self.error_index << 16) | self.index
    }
}

impl std::fmt::Display for ParsePosition {
    /// `toString()`: `java.text.ParsePosition[index=<i>,errorIndex=<e>]`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "java.text.ParsePosition[index={},errorIndex={}]", self.index, self.error_index)
    }
}

/// A growable vector of bits.
///
/// Equivalent to the subset of `java.util.BitSet` ICU calls (`clear()`,
/// `set(int)`, `get(int)`, `cardinality()`, `nextSetBit(int)`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BitSet {
    words: Vec<u64>,
}

impl BitSet {
    /// `new BitSet()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `clear()`.
    pub fn clear(&mut self) {
        self.words.clear();
    }

    /// `set(int bitIndex)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException("bitIndex < 0: <i>")` for a negative index.
    pub fn set(&mut self, bit_index: i32) {
        if bit_index < 0 {
            index_out_of_bounds(Some(format!("bitIndex < 0: {bit_index}")));
        }
        let w = (bit_index >> 6) as usize;
        if w >= self.words.len() {
            self.words.resize(w + 1, 0);
        }
        self.words[w] |= 1u64 << (bit_index & 63);
    }

    /// `get(int bitIndex)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException("bitIndex < 0: <i>")` for a negative index.
    pub fn get(&self, bit_index: i32) -> bool {
        if bit_index < 0 {
            index_out_of_bounds(Some(format!("bitIndex < 0: {bit_index}")));
        }
        let w = (bit_index >> 6) as usize;
        w < self.words.len() && (self.words[w] & (1u64 << (bit_index & 63))) != 0
    }

    /// `cardinality()`.
    pub fn cardinality(&self) -> i32 {
        self.words.iter().map(|w| w.count_ones() as i32).sum()
    }

    /// `nextSetBit(int fromIndex)`: -1 when there is none.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException("fromIndex < 0: <i>")` for a negative index.
    pub fn next_set_bit(&self, from_index: i32) -> i32 {
        if from_index < 0 {
            index_out_of_bounds(Some(format!("fromIndex < 0: {from_index}")));
        }
        let mut w = (from_index >> 6) as usize;
        if w >= self.words.len() {
            return -1;
        }
        let mut word = self.words[w] & (u64::MAX << (from_index & 63));
        loop {
            if word != 0 {
                return (w as i32) * 64 + word.trailing_zeros() as i32;
            }
            w += 1;
            if w == self.words.len() {
                return -1;
            }
            word = self.words[w];
        }
    }
}

/// `Character.codePointBefore(CharSequence seq, int index)`.
///
/// # Panics
///
/// Where Java throws `IndexOutOfBoundsException`: `index` outside `1..=len`.
pub fn code_point_before(seq: &[u16], index: i32) -> i32 {
    if index < 1 || index as usize > seq.len() {
        string_index_out_of_bounds(index);
    }
    let c2 = seq[index as usize - 1];
    if is_low_surrogate(c2) && index >= 2 {
        let c1 = seq[index as usize - 2];
        if is_high_surrogate(c1) {
            return to_code_point(c1, c2);
        }
    }
    c2 as i32
}

/// `String.offsetByCodePoints(int index, int codePointOffset)`.
///
/// # Panics
///
/// `IndexOutOfBoundsException` when `index` is outside the text or the
/// text has fewer code points than requested.
pub fn offset_by_code_points(seq: &[u16], index: i32, code_point_offset: i32) -> i32 {
    let length = seq.len() as i32;
    if index < 0 || index > length {
        index_out_of_bounds(None);
    }
    let mut x = index;
    if code_point_offset >= 0 {
        let mut i = 0;
        while x < length && i < code_point_offset {
            let c = seq[x as usize];
            x += 1;
            if is_high_surrogate(c) && x < length && is_low_surrogate(seq[x as usize]) {
                x += 1;
            }
            i += 1;
        }
        if i < code_point_offset {
            index_out_of_bounds(None);
        }
    } else {
        let mut i = code_point_offset;
        while x > 0 && i < 0 {
            x -= 1;
            let c = seq[x as usize];
            if is_low_surrogate(c) && x > 0 && is_high_surrogate(seq[x as usize - 1]) {
                x -= 1;
            }
            i += 1;
        }
        if i < 0 {
            index_out_of_bounds(None);
        }
    }
    x
}

/// `new RuntimeException(message)`.
pub fn runtime_exception(message: Option<String>) -> LuceneError {
    LuceneError::unchecked_with_message("java.lang.RuntimeException", message)
}

/// `new UnsupportedOperationException(message)`.
pub fn unsupported_operation(message: &str) -> LuceneError {
    LuceneError::unsupported_operation(message)
}

/// `Double.parseDouble(String s)`.
///
/// # Errors
///
/// `NumberFormatException` as the JDK throws it (`empty String`, or
/// `For input string: "<s>"`).
pub fn parse_double(s: &str) -> Result<f64> {
    if s.trim_matches(|c: char| c <= '\u{20}').is_empty() {
        return Err(LuceneError::number_format("empty String"));
    }
    rucene::util::java_fmt::parse_double(s).ok_or_else(|| LuceneError::number_format(format!("For input string: \"{s}\"")))
}
