//! Character utility helpers ported from `org.apache.lucene.analysis.CharacterUtils`.
//!
//! These helpers provide case folding, code-point conversion, and reader
//! buffering utilities used by tokenizers and token filters.

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
/// # Errors
///
/// Returns `LuceneError::IllegalArgument` if `buffer_size` is less than 2.
pub fn new_character_buffer(buffer_size: usize) -> Result<CharacterBuffer> {
    if buffer_size < 2 {
        return Err(LuceneError::IllegalArgument(
            "buffer_size must be >= 2".to_string(),
        ));
    }
    Ok(CharacterBuffer::new(vec!['\0'; buffer_size], 0, 0))
}

/// Lowercases each code point in `buffer[offset..limit]`.
///
/// Uses Rust's [`char::to_lowercase`] simple Unicode mapping. Unlike Java's
/// `Character.toLowerCase`, which always maps a single code point to a single
/// code point, Rust's simple mapping can produce multiple `char`s for some
/// input code points. A notable difference is U+0130 LATIN CAPITAL LETTER I
/// WITH DOT ABOVE: Java `Character.toLowerCase` returns U+0069 (`i`), while
/// Rust `char::to_lowercase` returns `i` followed by U+0307 COMBINING DOT
/// ABOVE. This function resizes `buffer` to accommodate any expansion.
///
/// Returns the new length of the lowercased region.
pub fn to_lower_case(buffer: &mut Vec<char>, offset: usize, limit: usize) -> usize {
    assert!(offset <= limit && limit <= buffer.len());
    let lowercased: Vec<char> = buffer[offset..limit]
        .iter()
        .flat_map(|c| c.to_lowercase())
        .collect();
    let new_len = lowercased.len();
    buffer.splice(offset..limit, lowercased);
    new_len
}

/// Uppercases each code point in `buffer[offset..limit]`.
///
/// Uses Rust's [`char::to_uppercase`] simple Unicode mapping. As with
/// [`to_lower_case`], this can expand the number of `char`s in the buffer and
/// will resize `buffer` accordingly.
pub fn to_upper_case(buffer: &mut Vec<char>, offset: usize, limit: usize) -> usize {
    assert!(offset <= limit && limit <= buffer.len());
    let uppercased: Vec<char> = buffer[offset..limit]
        .iter()
        .flat_map(|c| c.to_uppercase())
        .collect();
    let new_len = uppercased.len();
    buffer.splice(offset..limit, uppercased);
    new_len
}

/// Converts Java-style UTF-16 character data into Unicode code points.
///
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
/// In Java this method is careful not to split a surrogate pair across buffer
/// boundaries. Rust `char`s are complete scalar values, so no special handling
/// is required.
///
/// # Errors
///
/// Returns `LuceneError::IllegalArgument` if `num_chars` is out of range.
pub fn fill(
    buffer: &mut CharacterBuffer,
    reader: &mut dyn CharReader,
    num_chars: usize,
) -> Result<bool> {
    if num_chars < 2 || num_chars > buffer.buffer.len() {
        return Err(LuceneError::IllegalArgument(
            "num_chars must be >= 2 and <= buffer size".to_string(),
        ));
    }
    buffer.offset = 0;
    let read = read_fully(reader, &mut buffer.buffer, 0, num_chars)?;
    buffer.length = read;
    Ok(buffer.length == num_chars)
}

/// Convenience equivalent of [`fill(buffer, reader, buffer.buffer.len())`](fill).
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
        let new_len = to_lower_case(&mut buf, 0, len);
        assert_eq!(new_len, len);
        assert_eq!(buf.iter().collect::<String>(), "hello world");
    }

    #[test]
    fn to_upper_case_ascii() {
        let mut buf: Vec<char> = "hello world".chars().collect();
        let len = buf.len();
        let new_len = to_upper_case(&mut buf, 0, len);
        assert_eq!(new_len, len);
        assert_eq!(buf.iter().collect::<String>(), "HELLO WORLD");
    }

    #[test]
    fn to_lower_case_preserves_buffer_tail() {
        let mut buf: Vec<char> = "AbCdefG".chars().collect();
        let new_len = to_lower_case(&mut buf, 1, 4);
        assert_eq!(new_len, 3);
        assert_eq!(buf.iter().collect::<String>(), "AbcdefG");
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
