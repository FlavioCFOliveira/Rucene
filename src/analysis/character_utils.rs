//! Character utility helpers ported from `org.apache.lucene.analysis.CharacterUtils`.
//!
//! These helpers provide case folding, code-point conversion, and reader
//! buffering utilities used by tokenizers and token filters.
//!
//! Java's buffers are `char[]`, UTF-16 code units, where a supplementary
//! character takes two slots; the port's are `char`s, Unicode scalar values,
//! one slot each. The helpers therefore apply Java's per-code-point rules to
//! one slot at a time, and the conversions between code points and characters
//! are copies.

#![deny(unsafe_code)]

use crate::analysis::CharReader;
use crate::error::{LuceneError, Result};

/// A reusable character buffer used with [`fill`](crate::analysis::character_utils::fill).
///
/// Equivalent to `CharacterUtils.CharacterBuffer` in Lucene.
#[derive(Clone, Debug, Default)]
pub struct CharacterBuffer {
    buffer: Vec<char>,
    offset: usize,
    length: usize,
}

impl CharacterBuffer {
    /// Creates a buffer wrapping the supplied storage.
    pub fn new(buffer: Vec<char>, offset: usize, length: usize) -> Self {
        Self {
            buffer,
            offset,
            length,
        }
    }

    /// Returns the internal buffer.
    pub fn buffer(&self) -> &[char] {
        &self.buffer
    }

    /// Returns the data offset in the internal buffer.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Returns the length of valid data starting at [`Self::offset`].
    pub fn length(&self) -> usize {
        self.length
    }

    /// Resets the buffer to empty.
    pub fn reset(&mut self) {
        self.offset = 0;
        self.length = 0;
    }
}

/// Creates a new [`CharacterBuffer`] with a freshly allocated internal buffer.
///
/// Equivalent to `CharacterUtils.newCharacterBuffer(int)`.
///
/// # Errors
///
/// Returns `LuceneError::IllegalArgument` with Lucene's message,
/// `buffersize must be >= 2`, if `buffer_size` is less than 2.
pub fn new_character_buffer(buffer_size: usize) -> Result<CharacterBuffer> {
    if buffer_size < 2 {
        return Err(LuceneError::IllegalArgument(
            "buffersize must be >= 2".to_string(),
        ));
    }
    Ok(CharacterBuffer::new(vec!['\0'; buffer_size], 0, 0))
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

/// Converts Java-style UTF-16 character data into Unicode code points.
///
/// Equivalent to `CharacterUtils.toCodePoints(char[], int, int, int[], int)`.
/// Rust `char`s are already Unicode scalar values, so this is a direct copy.
///
/// # Errors
///
/// Returns `LuceneError::IllegalArgument` if `src_len` or the destination
/// capacity is invalid.
pub fn to_code_points(
    src: &[char],
    src_off: usize,
    src_len: usize,
    dest: &mut [i32],
    dest_off: usize,
) -> Result<usize> {
    if src_len > src.len().saturating_sub(src_off) {
        return Err(LuceneError::IllegalArgument(
            "src_len exceeds available source length".to_string(),
        ));
    }
    if dest_off + src_len > dest.len() {
        return Err(LuceneError::IllegalArgument(
            "destination buffer too small".to_string(),
        ));
    }
    for i in 0..src_len {
        dest[dest_off + i] = src[src_off + i] as i32;
    }
    Ok(src_len)
}

/// Converts Unicode code points into Java-style UTF-16 characters.
///
/// Equivalent to `CharacterUtils.toChars(int[], int, int, char[], int)`.
/// Rust `char`s already cover the full Unicode scalar range, so this is a
/// direct copy.
///
/// # Errors
///
/// Returns `LuceneError::IllegalArgument` if `src_len` or the destination
/// capacity is invalid, or if a code point is not a valid Unicode scalar value.
pub fn to_chars(
    src: &[i32],
    src_off: usize,
    src_len: usize,
    dest: &mut [char],
    dest_off: usize,
) -> Result<usize> {
    if src_len > src.len().saturating_sub(src_off) {
        return Err(LuceneError::IllegalArgument(
            "src_len exceeds available source length".to_string(),
        ));
    }
    if dest_off + src_len > dest.len() {
        return Err(LuceneError::IllegalArgument(
            "destination buffer too small".to_string(),
        ));
    }
    for i in 0..src_len {
        let cp = src[src_off + i];
        let c = char::from_u32(cp as u32)
            .ok_or_else(|| LuceneError::IllegalArgument(format!("invalid code point: {cp}")))?;
        dest[dest_off + i] = c;
    }
    Ok(src_len)
}

/// Fills `buffer` with up to `num_chars` characters from `reader`.
///
/// Equivalent to `CharacterUtils.fill(CharacterBuffer, Reader, int)`. In Java
/// this method is careful not to split a surrogate pair across buffer
/// boundaries. Rust `char`s are complete scalar values, so no special handling
/// is required.
///
/// # Errors
///
/// Returns `LuceneError::IllegalArgument` with Lucene's message,
/// `numChars must be >= 2 and <= the buffer size`, if `num_chars` is out of
/// range.
pub fn fill(
    buffer: &mut CharacterBuffer,
    reader: &mut dyn CharReader,
    num_chars: usize,
) -> Result<bool> {
    if num_chars < 2 || num_chars > buffer.buffer.len() {
        return Err(LuceneError::IllegalArgument(
            "numChars must be >= 2 and <= the buffer size".to_string(),
        ));
    }
    buffer.offset = 0;
    let read = read_fully(reader, &mut buffer.buffer, 0, num_chars)?;
    buffer.length = read;
    Ok(buffer.length == num_chars)
}

/// Convenience equivalent of [`fill(buffer, reader, buffer.buffer.len())`](fill).
///
/// Equivalent to the overload `CharacterUtils.fill(CharacterBuffer, Reader)`.
pub fn fill_buffer(buffer: &mut CharacterBuffer, reader: &mut dyn CharReader) -> Result<bool> {
    fill(buffer, reader, buffer.buffer.len())
}

fn read_fully(
    reader: &mut dyn CharReader,
    dest: &mut [char],
    offset: usize,
    len: usize,
) -> Result<usize> {
    let mut read = 0;
    while read < len {
        let n = reader.read(&mut dest[offset + read..offset + len])?;
        if n == 0 {
            break;
        }
        read += n;
    }
    Ok(read)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_character_buffer_rejects_small_size() {
        assert!(matches!(
            new_character_buffer(1),
            Err(LuceneError::IllegalArgument(_))
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
            Err(LuceneError::IllegalArgument(message)) => {
                assert_eq!(message, "buffersize must be >= 2")
            }
            other => panic!("expected the size error, got {other:?}"),
        }
        let mut buffer = new_character_buffer(4).unwrap();
        let mut reader = crate::analysis::StringCharReader::new("x");
        match fill(&mut buffer, &mut reader, 5) {
            Err(LuceneError::IllegalArgument(message)) => {
                assert_eq!(message, "numChars must be >= 2 and <= the buffer size")
            }
            other => panic!("expected the count error, got {other:?}"),
        }
    }

    #[test]
    fn to_code_points_round_trip() {
        let src: Vec<char> = "abc".chars().collect();
        let mut dest = [0_i32; 3];
        assert_eq!(to_code_points(&src, 0, 3, &mut dest, 0).unwrap(), 3);
        assert_eq!(&dest, &[97, 98, 99]);

        let mut chars = ['\0'; 3];
        assert_eq!(to_chars(&dest, 0, 3, &mut chars, 0).unwrap(), 3);
        assert_eq!(chars.iter().collect::<String>(), "abc");
    }

    #[test]
    fn fill_reads_into_buffer() {
        let mut reader = crate::analysis::StringCharReader::new("hello");
        let mut buf = new_character_buffer(5).unwrap();
        let full = fill(&mut buf, &mut reader, 5).unwrap();
        assert!(full);
        assert_eq!(buf.length(), 5);
        assert_eq!(buf.buffer()[0..5].iter().collect::<String>(), "hello");

        let full = fill(&mut buf, &mut reader, 5).unwrap();
        assert!(!full);
        assert_eq!(buf.length(), 0);
    }
}
