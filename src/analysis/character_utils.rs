//! Character utility helpers ported from `org.apache.lucene.analysis.CharacterUtils`.
//!
//! These helpers provide case folding, code-point conversion, and reader
//! buffering utilities used by tokenizers and token filters.
//!
//! Java's buffers are `char[]`, UTF-16 code units, where a supplementary
//! character takes two slots; the port's are `char`s, Unicode scalar values,
//! one slot each. The helpers therefore apply Java's per-code-point rules to
//! one slot at a time, and the conversions between code points and characters
//! are copies: `to_code_points` and `to_chars` take and return counts of
//! slots, where Java's `srcLen` of `toCodePoints` and the result of `toChars`
//! count UTF-16 units. [`fill`] keeps Java's budget, which counts UTF-16
//! units.

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
    /// Characters read from the reader that did not fit the UTF-16 budget of
    /// the last [`fill`]; the next `fill` installs them first.
    ///
    /// Java's `lastTrailingHighSurrogate` holds the high surrogate of a pair
    /// that would end a full buffer, and the characters past the budget stay
    /// in Java's reader, which hands out `char`s. A [`CharReader`] hands out
    /// whole `char`s, so a read can return more UTF-16 units than requested;
    /// those characters, and a supplementary character straddling the budget,
    /// wait here.
    pending: Vec<char>,
}

impl CharacterBuffer {
    /// Creates a buffer wrapping the supplied storage.
    pub fn new(buffer: Vec<char>, offset: usize, length: usize) -> Self {
        Self {
            buffer,
            offset,
            length,
            pending: Vec::new(),
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
    ///
    /// The length counts the buffer's slots, `char`s, where Java's
    /// `getLength()` counts UTF-16 code units: it bounds the data a caller
    /// walks, which is what Java's callers use it for.
    pub fn length(&self) -> usize {
        self.length
    }

    /// Resets the buffer to empty.
    pub fn reset(&mut self) {
        self.offset = 0;
        self.length = 0;
        self.pending.clear();
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

/// Fills `buffer` with up to `num_chars` UTF-16 code units of characters
/// from `reader`.
///
/// Equivalent to `CharacterUtils.fill(CharacterBuffer, Reader, int)`
/// (`CharacterUtils.java:140-174`). `num_chars` counts UTF-16 code units, as
/// Java's `char`s do, so a supplementary character takes two of them. A
/// supplementary character that would straddle the budget is held back for
/// the next call, as Java holds back the high surrogate that would end a full
/// buffer, and the call still reports a full buffer; the buffer then holds
/// `num_chars - 1` units.
///
/// # Errors
///
/// Returns `LuceneError::IllegalArgument` with Lucene's message,
/// `numChars must be >= 2 and <= the buffer size`, if `num_chars` is out of
/// range, and any error of `reader`.
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
    // Install the characters a previous call read but could not hold.
    let mut incoming = std::mem::take(&mut buffer.pending);
    let mut length = 0;
    let mut units = 0;
    loop {
        for (i, &c) in incoming.iter().enumerate() {
            let next = units + c.len_utf16();
            if next > num_chars {
                // Past the budget: Java's reader would still hold these. A
                // supplementary character crossing the budget fills Java's
                // buffer with its high surrogate, which Java then holds back.
                if units < num_chars {
                    units = num_chars;
                }
                buffer.pending.extend_from_slice(&incoming[i..]);
                break;
            }
            buffer.buffer[length] = c;
            length += 1;
            units = next;
        }
        if units >= num_chars {
            break;
        }
        // `readFully(reader, charBuffer, offset, numChars - offset)`: each
        // character takes at least one unit, so asking for the remaining
        // units in characters never misses one Java would read.
        let want = num_chars - units;
        let mut scratch = vec!['\0'; want];
        let read = reader.read(&mut scratch)?;
        if read == 0 {
            break;
        }
        scratch.truncate(read);
        incoming = scratch;
    }
    buffer.length = length;
    Ok(units == num_chars)
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

    fn data(buf: &CharacterBuffer) -> String {
        buf.buffer()[buf.offset()..buf.offset() + buf.length()]
            .iter()
            .collect()
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
