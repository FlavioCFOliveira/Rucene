//! Character utility helpers ported from `org.apache.lucene.analysis.CharacterUtils`.
//!
//! These helpers provide case folding, code-point conversion, and reader
//! buffering utilities used by tokenizers and token filters.
//!
//! The case-folding helpers work on `char` buffers, Unicode scalar values,
//! one slot per code point, where Java's work on `char[]`, UTF-16 code
//! units; they apply Java's per-code-point rules to one slot at a time.
//! [`CharacterBuffer`], [`fill`], [`to_code_points`], [`to_chars`] and
//! [`to_lower_case_utf16`] hold UTF-16 code units (`u16`), as Java's `char[]`
//! does, and [`CharReader`] hands out UTF-16 units, as Java's `Reader` does.

#![deny(unsafe_code)]

use crate::analysis::CharReader;
use crate::error::{LuceneError, Result};

/// A reusable character buffer used with [`fill`](crate::analysis::character_utils::fill).
///
/// Equivalent to `CharacterUtils.CharacterBuffer` in Lucene: the buffer holds
/// UTF-16 code units, as Java's `char[]` does.
#[derive(Clone, Debug, Default)]
pub struct CharacterBuffer {
    buffer: Vec<u16>,
    offset: usize,
    length: usize,
    /// `lastTrailingHighSurrogate` (`CharacterUtils.java:218`): the high
    /// surrogate that ended the last full buffer, `0` for none.
    last_trailing_high_surrogate: u16,
}

impl CharacterBuffer {
    /// Creates a buffer wrapping the supplied storage.
    pub fn new(buffer: Vec<u16>, offset: usize, length: usize) -> Self {
        Self {
            buffer,
            offset,
            length,
            last_trailing_high_surrogate: 0,
        }
    }

    /// Returns the internal buffer: `getBuffer()`.
    pub fn buffer(&self) -> &[u16] {
        &self.buffer
    }

    /// Returns the data offset in the internal buffer: `getOffset()`.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Returns the length, in UTF-16 code units, of valid data starting at
    /// [`Self::offset`]: `getLength()`.
    pub fn length(&self) -> usize {
        self.length
    }

    /// Resets the buffer to empty: `reset()`.
    pub fn reset(&mut self) {
        self.offset = 0;
        self.length = 0;
        self.last_trailing_high_surrogate = 0;
    }
}

/// Creates a new [`CharacterBuffer`] with a freshly allocated internal buffer.
///
/// Equivalent to `CharacterUtils.newCharacterBuffer(int)`.
///
/// # Errors
///
/// Returns `LuceneErrorKind::IllegalArgument` with Lucene's message,
/// `buffersize must be >= 2`, if `buffer_size` is less than 2.
pub fn new_character_buffer(buffer_size: usize) -> Result<CharacterBuffer> {
    if buffer_size < 2 {
        return Err(LuceneError::illegal_argument(
            "buffersize must be >= 2".to_string(),
        ));
    }
    Ok(CharacterBuffer::new(vec![0u16; buffer_size], 0, 0))
}

/// Lowercases each code point in `buffer[offset..limit]`, in place.
///
/// Equivalent to `CharacterUtils.toLowerCase(char[], int, int)`
/// (`CharacterUtils.java:53-61`), which replaces every code point with
/// `Character.toLowerCase(int)` of it: Unicode's *simple* lowercase mapping,
/// one code point for one, so U+0130 becomes `i` and U+1E9E becomes U+00DF
/// (Rust's [`char::to_lowercase`] applies the full mapping instead, which
/// turns U+0130 into two characters). The mapping is
/// [`case_folding::to_lower_case`](crate::util::automaton::case_folding::to_lower_case),
/// generated from the JDK.
pub fn to_lower_case(buffer: &mut [char], offset: usize, limit: usize) {
    debug_assert!(buffer.len() >= limit);
    debug_assert!(offset <= buffer.len());
    for c in &mut buffer[offset..limit] {
        *c = simple_case(*c, crate::util::automaton::case_folding::to_lower_case);
    }
}

/// Lowercases each code point in the UTF-16 units `buffer[offset..limit]`,
/// in place.
///
/// Equivalent to `CharacterUtils.toLowerCase(char[], int, int)`
/// (`CharacterUtils.java:53-61`) over Java's `char[]`, UTF-16 code units:
/// `i += Character.toChars(Character.toLowerCase(Character.codePointAt(buffer,
/// i, limit)), buffer, i)`, so a lone surrogate is lower-cased as a code point
/// of its own (it maps to itself). [`to_lower_case`] is the same operation
/// over a `char` buffer.
pub fn to_lower_case_utf16(buffer: &mut [u16], offset: usize, limit: usize) {
    debug_assert!(buffer.len() >= limit);
    debug_assert!(offset <= buffer.len());
    let mut i = offset;
    while i < limit {
        let lower =
            crate::util::automaton::case_folding::to_lower_case(code_point_at(buffer, i, limit));
        i += to_chars(&[lower], 0, 1, buffer, i)
            .expect("INVARIANT: Character.toLowerCase maps a code point to a code point");
    }
}

/// Uppercases each code point in the UTF-16 units `buffer[offset..limit]`,
/// in place.
///
/// Equivalent to `CharacterUtils.toUpperCase(char[], int, int)`
/// (`CharacterUtils.java:71-79`) over Java's `char[]`, with
/// `Character.toUpperCase(int)`, the simple mapping. [`to_upper_case`] is the
/// same operation over a `char` buffer.
pub fn to_upper_case_utf16(buffer: &mut [u16], offset: usize, limit: usize) {
    debug_assert!(buffer.len() >= limit);
    debug_assert!(offset <= buffer.len());
    let mut i = offset;
    while i < limit {
        let upper =
            crate::util::automaton::case_folding::to_upper_case(code_point_at(buffer, i, limit));
        i += to_chars(&[upper], 0, 1, buffer, i)
            .expect("INVARIANT: Character.toUpperCase maps a code point to a code point");
    }
}

/// `Character.codePointAt(char[] a, int index, int limit)`: the code point at
/// `index` of the UTF-16 units `a`, joining a surrogate pair that ends before
/// `limit`; any other unit, a lone surrogate included, is taken as is.
///
/// # Panics
///
/// Panics where Java throws `IndexOutOfBoundsException`: `index` at or past
/// `limit` or the end of `a`.
pub fn code_point_at(a: &[u16], index: usize, limit: usize) -> i32 {
    assert!(
        index < limit && limit <= a.len(),
        "IndexOutOfBoundsException"
    );
    let c1 = a[index];
    if is_high_surrogate(c1) && index + 1 < limit && is_low_surrogate(a[index + 1]) {
        0x10000 + ((i32::from(c1) - 0xD800) << 10) + (i32::from(a[index + 1]) - 0xDC00)
    } else {
        i32::from(c1)
    }
}

/// `Character.charCount(int codePoint)`: 2 for a supplementary code point,
/// 1 otherwise.
pub fn char_count(code_point: i32) -> usize {
    if code_point >= 0x10000 {
        2
    } else {
        1
    }
}

/// `new String(char[])` over UTF-16 units.
///
/// **Divergence from Lucene 10.5.0.** A Java `String` holds any UTF-16
/// sequence; a Rust `String` cannot hold a lone surrogate, which becomes
/// U+FFFD here. Every well-formed sequence renders as Java's.
pub fn utf16_to_string(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

/// Uppercases each code point in `buffer[offset..limit]`, in place.
///
/// Equivalent to `CharacterUtils.toUpperCase(char[], int, int)`
/// (`CharacterUtils.java:71-79`), with `Character.toUpperCase(int)`, the
/// simple uppercase mapping: U+00DF stays U+00DF, where Rust's
/// [`char::to_uppercase`] gives `SS`.
pub fn to_upper_case(buffer: &mut [char], offset: usize, limit: usize) {
    debug_assert!(buffer.len() >= limit);
    debug_assert!(offset <= buffer.len());
    for c in &mut buffer[offset..limit] {
        *c = simple_case(*c, crate::util::automaton::case_folding::to_upper_case);
    }
}

/// Applies a simple (one code point for one) case mapping to `c`.
fn simple_case(c: char, map: fn(i32) -> i32) -> char {
    char::from_u32(map(c as i32) as u32)
        .expect("INVARIANT: a simple case mapping maps a scalar value to a scalar value")
}

/// `Character.isHighSurrogate(char)`.
fn is_high_surrogate(unit: u16) -> bool {
    (0xD800..=0xDBFF).contains(&unit)
}

/// `Character.isLowSurrogate(char)`.
fn is_low_surrogate(unit: u16) -> bool {
    (0xDC00..=0xDFFF).contains(&unit)
}

/// Converts a sequence of Java characters (UTF-16 code units) to a sequence
/// of Unicode code points, returning the number of code points written to
/// `dest`.
///
/// Equivalent to `CharacterUtils.toCodePoints(char[], int, int, int[], int)`
/// (`CharacterUtils.java:85-97`): each code point is
/// `Character.codePointAt(src, srcOff + i, srcOff + srcLen)`, a surrogate
/// pair inside the range joined, any other unit (a lone surrogate included)
/// taken as is. Java's `IllegalArgumentException` for a negative `srcLen`
/// has no counterpart, the length being a `usize`.
///
/// # Errors
///
/// None; the `Result` is kept for the callers.
///
/// # Panics
///
/// Panics where Java throws `ArrayIndexOutOfBoundsException`: a range past
/// the end of `src` or `dest`.
pub fn to_code_points(
    src: &[u16],
    src_off: usize,
    src_len: usize,
    dest: &mut [i32],
    dest_off: usize,
) -> Result<usize> {
    let limit = src_off + src_len;
    let mut code_point_count = 0;
    let mut i = 0;
    while i < src_len {
        let index = src_off + i;
        let c1 = src[index];
        let cp = if is_high_surrogate(c1) && index + 1 < limit && is_low_surrogate(src[index + 1]) {
            0x10000 + (((c1 as i32) - 0xD800) << 10) + ((src[index + 1] as i32) - 0xDC00)
        } else {
            c1 as i32
        };
        let char_count = if cp >= 0x10000 { 2 } else { 1 };
        dest[dest_off + code_point_count] = cp;
        code_point_count += 1;
        i += char_count;
    }
    Ok(code_point_count)
}

/// Converts a sequence of Unicode code points to a sequence of Java
/// characters (UTF-16 code units), returning the number of units written to
/// `dest`.
///
/// Equivalent to `CharacterUtils.toChars(int[], int, int, char[], int)`
/// (`CharacterUtils.java:104-113`), with `Character.toChars(int, char[],
/// int)` for each code point.
///
/// # Errors
///
/// Returns `LuceneErrorKind::IllegalArgument` with the JDK's message, `Not a
/// valid Unicode code point: 0x…`, for a value outside `0..=0x10FFFF`.
///
/// # Panics
///
/// Panics where Java throws `ArrayIndexOutOfBoundsException`: a range past
/// the end of `src` or `dest`.
pub fn to_chars(
    src: &[i32],
    src_off: usize,
    src_len: usize,
    dest: &mut [u16],
    dest_off: usize,
) -> Result<usize> {
    let mut written = 0;
    for i in 0..src_len {
        let cp = src[src_off + i];
        if (0..0x10000).contains(&cp) {
            dest[dest_off + written] = cp as u16;
            written += 1;
        } else if (0x10000..=0x10FFFF).contains(&cp) {
            let v = cp - 0x10000;
            dest[dest_off + written] = (0xD800 + (v >> 10)) as u16;
            dest[dest_off + written + 1] = (0xDC00 + (v & 0x3FF)) as u16;
            written += 2;
        } else {
            return Err(LuceneError::illegal_argument(format!(
                "Not a valid Unicode code point: 0x{:X}",
                cp as u32
            )));
        }
    }
    Ok(written)
}

/// Fills `buffer` with characters read from `reader`.
///
/// Equivalent to `CharacterUtils.fill(CharacterBuffer, Reader, int)`
/// (`CharacterUtils.java:140-174`): tries to read `num_chars` UTF-16 units
/// into the buffer, from offset `0`, after the high surrogate the previous
/// call held back. A full buffer that ends with a high surrogate holds it
/// back for the next call, so the buffer then has `num_chars - 1` units.
/// Returns `false` if and only if the reader was exhausted while trying to
/// fill the buffer.
///
/// # Errors
///
/// Returns `LuceneErrorKind::IllegalArgument` with Lucene's message,
/// `numChars must be >= 2 and <= the buffer size`, if `num_chars` is out of
/// range, and any error of `reader`.
pub fn fill(
    buffer: &mut CharacterBuffer,
    reader: &mut dyn CharReader,
    num_chars: usize,
) -> Result<bool> {
    debug_assert!(buffer.buffer.len() >= 2);
    if num_chars < 2 || num_chars > buffer.buffer.len() {
        return Err(LuceneError::illegal_argument(
            "numChars must be >= 2 and <= the buffer size".to_string(),
        ));
    }
    buffer.offset = 0;

    // Install the previously saved ending high surrogate:
    let offset = if buffer.last_trailing_high_surrogate != 0 {
        buffer.buffer[0] = buffer.last_trailing_high_surrogate;
        buffer.last_trailing_high_surrogate = 0;
        1
    } else {
        0
    };

    let read = read_fully(buffer, reader, offset, num_chars - offset)?;

    buffer.length = offset + read;
    let result = buffer.length == num_chars;
    if buffer.length < num_chars {
        // We failed to fill the buffer. Even if the last char is a high
        // surrogate, there is nothing we can do
        return Ok(result);
    }

    if is_high_surrogate(buffer.buffer[buffer.length - 1]) {
        buffer.length -= 1;
        buffer.last_trailing_high_surrogate = buffer.buffer[buffer.length];
    }
    Ok(result)
}

/// `readFully(Reader, char[], int, int)` (`CharacterUtils.java:181-191`)
/// into `buffer`'s storage.
fn read_fully(
    buffer: &mut CharacterBuffer,
    reader: &mut dyn CharReader,
    offset: usize,
    len: usize,
) -> Result<usize> {
    let mut read = 0;
    while read < len {
        let start = offset + read;
        let r = reader.read(&mut buffer.buffer[start..offset + len])?;
        if r == 0 {
            // `if (r == -1) break;`: a `CharReader` answers 0 at the end of
            // the stream, and the request is never empty here.
            break;
        }
        read += r;
    }
    Ok(read)
}

/// Convenience equivalent of [`fill(buffer, reader, buffer.buffer.len())`](fill).
///
/// Equivalent to the overload `CharacterUtils.fill(CharacterBuffer, Reader)`.
pub fn fill_buffer(buffer: &mut CharacterBuffer, reader: &mut dyn CharReader) -> Result<bool> {
    fill(buffer, reader, buffer.buffer.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::LuceneErrorKind;

    #[test]
    fn new_character_buffer_rejects_small_size() {
        assert!(matches!(
            new_character_buffer(1),
            Err(LuceneError {
                kind: LuceneErrorKind::IllegalArgument { .. },
                ..
            })
        ));
    }

    #[test]
    fn to_lower_case_ascii() {
        let mut buf: Vec<char> = "HELLO WORLD".chars().collect();
        let len = buf.len();
        to_lower_case(&mut buf, 0, len);
        assert_eq!(buf.iter().collect::<String>(), "hello world");
    }

    #[test]
    fn to_upper_case_ascii() {
        let mut buf: Vec<char> = "hello world".chars().collect();
        let len = buf.len();
        to_upper_case(&mut buf, 0, len);
        assert_eq!(buf.iter().collect::<String>(), "HELLO WORLD");
    }

    #[test]
    fn to_lower_case_preserves_buffer_tail() {
        let mut buf: Vec<char> = "AbCdefG".chars().collect();
        to_lower_case(&mut buf, 1, 4);
        assert_eq!(buf.iter().collect::<String>(), "AbcdefG");
    }

    /// `MiscOracle` O3 (`oracle/h2/MiscOracle.java`): Java's
    /// `CharacterUtils.toLowerCase` and `toUpperCase` apply the simple
    /// mappings, one code point for one.
    #[test]
    fn case_mappings_are_javas_simple_mappings() {
        let mut lower: Vec<char> =
            "\u{130}STANBUL \u{3A3}\u{391}\u{3A3} \u{1E9E} \u{10400} \u{1C5}"
                .chars()
                .collect();
        let len = lower.len();
        to_lower_case(&mut lower, 0, len);
        assert_eq!(
            lower.iter().collect::<String>(),
            "istanbul \u{3C3}\u{3B1}\u{3C3} \u{DF} \u{10428} \u{1C6}"
        );
        let mut upper: Vec<char> = "stra\u{DF}e \u{1C6} \u{149} \u{10428} \u{B5} \u{131}"
            .chars()
            .collect();
        let len = upper.len();
        to_upper_case(&mut upper, 0, len);
        assert_eq!(
            upper.iter().collect::<String>(),
            "STRA\u{DF}E \u{1C4} \u{149} \u{10400} \u{39C} I"
        );
    }

    /// `MiscOracle` O3: the buffer helpers refuse a bad size with Java's
    /// messages.
    #[test]
    fn buffer_errors_carry_javas_messages() {
        match new_character_buffer(1) {
            Err(LuceneError {
                kind: LuceneErrorKind::IllegalArgument { message, .. },
                ..
            }) => {
                assert_eq!(message, "buffersize must be >= 2")
            }
            other => panic!("expected the size error, got {other:?}"),
        }
        let mut buffer = new_character_buffer(4).unwrap();
        let mut reader = crate::analysis::StringCharReader::new("x");
        match fill(&mut buffer, &mut reader, 5) {
            Err(LuceneError {
                kind: LuceneErrorKind::IllegalArgument { message, .. },
                ..
            }) => {
                assert_eq!(message, "numChars must be >= 2 and <= the buffer size")
            }
            other => panic!("expected the count error, got {other:?}"),
        }
    }

    #[test]
    fn to_code_points_round_trip() {
        let src: Vec<u16> = "abc".encode_utf16().collect();
        let mut dest = [0_i32; 3];
        assert_eq!(to_code_points(&src, 0, 3, &mut dest, 0).unwrap(), 3);
        assert_eq!(&dest, &[97, 98, 99]);

        let mut chars = [0u16; 3];
        assert_eq!(to_chars(&dest, 0, 3, &mut chars, 0).unwrap(), 3);
        assert_eq!(String::from_utf16_lossy(&chars), "abc");
    }

    #[test]
    fn fill_reads_into_buffer() {
        let mut reader = crate::analysis::StringCharReader::new("hello");
        let mut buf = new_character_buffer(5).unwrap();
        let full = fill(&mut buf, &mut reader, 5).unwrap();
        assert!(full);
        assert_eq!(buf.length(), 5);
        assert_eq!(String::from_utf16_lossy(&buf.buffer()[0..5]), "hello");

        let full = fill(&mut buf, &mut reader, 5).unwrap();
        assert!(!full);
        assert_eq!(buf.length(), 0);
    }

    fn data(buf: &CharacterBuffer) -> String {
        String::from_utf16_lossy(&buf.buffer()[buf.offset()..buf.offset() + buf.length()])
    }

    /// `TestCharacterUtils.testFillNoHighSurrogate`.
    #[test]
    fn fill_no_high_surrogate() {
        let mut reader = crate::analysis::StringCharReader::new("helloworld");
        let mut buf = new_character_buffer(6).unwrap();
        assert!(fill_buffer(&mut buf, &mut reader).unwrap());
        assert_eq!(buf.offset(), 0);
        assert_eq!(buf.length(), 6);
        assert_eq!(data(&buf), "hellow");
        assert!(!fill_buffer(&mut buf, &mut reader).unwrap());
        assert_eq!(buf.length(), 4);
        assert_eq!(buf.offset(), 0);
        assert_eq!(data(&buf), "orld");
        assert!(!fill_buffer(&mut buf, &mut reader).unwrap());
    }

    /// `TestCharacterUtils.testFill` over paired surrogates only (a `&str`
    /// cannot hold the lone ones of Lucene's input): the budget counts UTF-16
    /// units, and a supplementary character straddling it waits for the next
    /// call while the call reports a full buffer.
    #[test]
    fn fill_counts_utf16_units_and_holds_a_straddling_pair() {
        let mut reader = crate::analysis::StringCharReader::new("1234\u{1041C}789123\u{1041C}");
        let mut buf = new_character_buffer(5).unwrap();
        assert!(fill_buffer(&mut buf, &mut reader).unwrap());
        assert_eq!(data(&buf), "1234");
        assert!(fill_buffer(&mut buf, &mut reader).unwrap());
        assert_eq!(data(&buf), "\u{1041C}789");
        assert!(fill_buffer(&mut buf, &mut reader).unwrap());
        assert_eq!(data(&buf), "123\u{1041C}");
        assert!(!fill_buffer(&mut buf, &mut reader).unwrap());
        assert_eq!(buf.length(), 0);
    }

    /// A read that returns more UTF-16 units than the budget keeps the
    /// characters past it for the next call, as Java's reader would.
    #[test]
    fn fill_keeps_characters_read_past_the_budget() {
        let text = "\u{1F4A9}\u{1F4A9}\u{1F4A9}ab";
        let mut reader = crate::analysis::StringCharReader::new(text);
        let mut buf = new_character_buffer(4).unwrap();
        assert!(fill_buffer(&mut buf, &mut reader).unwrap());
        assert_eq!(data(&buf), "\u{1F4A9}\u{1F4A9}");
        assert!(fill_buffer(&mut buf, &mut reader).unwrap());
        assert_eq!(data(&buf), "\u{1F4A9}ab");
        assert!(!fill_buffer(&mut buf, &mut reader).unwrap());
        assert_eq!(buf.length(), 0);
        buf.reset();
        assert_eq!(buf.length(), 0);
    }
}
