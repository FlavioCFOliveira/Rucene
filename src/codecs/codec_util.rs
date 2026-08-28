//! Codec header/footer helpers ported from `org.apache.lucene.codecs.CodecUtil`.
//!
//! Every Lucene index file begins with a versioned codec header and ends with a
//! CRC-32 footer. This module writes and validates those envelopes so that
//! Rucene produces byte-compatible index files with Apache Lucene Core 10.5.0.
//!
//! # Layout
//!
//! CodecHeader --&gt; Magic,CodecName,Version
//!
//! IndexHeader --&gt; CodecHeader,ObjectID,ObjectSuffix
//!
//! CodecFooter --&gt; Magic,AlgorithmID,Checksum

#![deny(unsafe_code)]

use crate::{
    error::LuceneError,
    store::{
        BufferedChecksumIndexInput, ChecksumIndexInput, DataInput, DataOutput, IndexInput,
        IndexOutput,
    },
    util::string_helper::ID_LENGTH,
    Result,
};

// -----------------------------------------------------------------------------
// Magic constants
// -----------------------------------------------------------------------------

/// Constant that identifies the start of a codec header.
///
/// Equivalent to `CodecUtil.CODEC_MAGIC` in Lucene.
pub const CODEC_MAGIC: i32 = 0x3fd76c17;

/// Constant that identifies the start of a codec footer.
///
/// Equivalent to `CodecUtil.FOOTER_MAGIC` in Lucene (`~CODEC_MAGIC`).
pub const FOOTER_MAGIC: i32 = !CODEC_MAGIC;

/// Length in bytes of a codec footer.
///
/// Equivalent to `CodecUtil.footerLength()`.
pub const fn footer_length() -> i32 {
    16
}

// -----------------------------------------------------------------------------
// Header writing
// -----------------------------------------------------------------------------

/// Writes a codec header.
///
/// Layout: `CODEC_MAGIC` (big-endian int), codec name (Lucene string), version
/// (big-endian int).
///
/// # Errors
///
/// Returns `LuceneError::IllegalArgument` if `codec` is not simple ASCII or is
/// 128 characters or longer.
///
/// Equivalent to `CodecUtil.writeHeader`.
pub fn write_header(out: &mut dyn DataOutput, codec: &str, version: i32) -> Result<()> {
    validate_codec_name(codec)?;
    write_be_int(out, CODEC_MAGIC)?;
    out.write_string(codec)?;
    write_be_int(out, version)?;
    Ok(())
}

/// Writes an index header.
///
/// Layout: codec header, 16-byte object id, suffix length byte, suffix bytes.
///
/// # Errors
///
/// Returns `LuceneError::IllegalArgument` if `codec` or `suffix` are not
/// simple ASCII, if `id` is not exactly 16 bytes, or if `suffix` is 256
/// characters or longer.
///
/// Equivalent to `CodecUtil.writeIndexHeader`.
pub fn write_index_header(
    out: &mut dyn DataOutput,
    codec: &str,
    version: i32,
    id: &[u8],
    suffix: &str,
) -> Result<()> {
    if id.len() != ID_LENGTH {
        return Err(LuceneError::IllegalArgument(format!(
            "invalid id length: expected {}, got {}",
            ID_LENGTH,
            id.len()
        )));
    }
    write_header(out, codec, version)?;
    out.write_bytes(id, 0, id.len())?;
    validate_suffix(suffix)?;
    out.write_byte(suffix.len() as u8)?;
    out.write_bytes(suffix.as_bytes(), 0, suffix.len())?;
    Ok(())
}

/// Returns the length of a codec header for `codec`.
///
/// Equivalent to `CodecUtil.headerLength`.
pub fn header_length(codec: &str) -> i32 {
    9 + codec.len() as i32
}

/// Returns the length of an index header for `codec` and `suffix`.
///
/// Equivalent to `CodecUtil.indexHeaderLength`.
pub fn index_header_length(codec: &str, suffix: &str) -> i32 {
    header_length(codec) + ID_LENGTH as i32 + 1 + suffix.len() as i32
}

fn validate_codec_name(codec: &str) -> Result<()> {
    let bytes = codec.as_bytes();
    if bytes.len() != codec.len() || bytes.len() >= 128 {
        return Err(LuceneError::IllegalArgument(format!(
            "codec must be simple ASCII, less than 128 characters in length [got {}]",
            codec
        )));
    }
    for &b in bytes {
        if b > 0x7f {
            return Err(LuceneError::IllegalArgument(format!(
                "codec must be simple ASCII, less than 128 characters in length [got {}]",
                codec
            )));
        }
    }
    Ok(())
}

fn validate_suffix(suffix: &str) -> Result<()> {
    let bytes = suffix.as_bytes();
    if bytes.len() != suffix.len() || bytes.len() >= 256 {
        return Err(LuceneError::IllegalArgument(format!(
            "suffix must be simple ASCII, less than 256 characters in length [got {}]",
            suffix
        )));
    }
    for &b in bytes {
        if b > 0x7f {
            return Err(LuceneError::IllegalArgument(format!(
                "suffix must be simple ASCII, less than 256 characters in length [got {}]",
                suffix
            )));
        }
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Header validation
// -----------------------------------------------------------------------------

/// Reads and validates a codec header.
///
/// # Errors
///
/// Returns `LuceneError::CorruptIndex` if the magic or codec name do not match,
/// and `LuceneError::IndexFormatNotSupported` if the version is outside the
/// supported range.
///
/// Equivalent to `CodecUtil.checkHeader`.
pub fn check_header(
    input: &mut dyn DataInput,
    codec: &str,
    min_version: i32,
    max_version: i32,
) -> Result<i32> {
    let actual_header = read_be_int(input)?;
    if actual_header != CODEC_MAGIC {
        return Err(LuceneError::CorruptIndex(format!(
            "codec header mismatch: actual header={} vs expected header={}",
            actual_header, CODEC_MAGIC
        )));
    }
    check_header_no_magic(input, codec, min_version, max_version)
}

/// Like [`check_header`], but assumes the magic int has already been consumed.
///
/// Equivalent to `CodecUtil.checkHeaderNoMagic`.
pub fn check_header_no_magic(
    input: &mut dyn DataInput,
    codec: &str,
    min_version: i32,
    max_version: i32,
) -> Result<i32> {
    let actual_codec = input.read_string()?;
    if actual_codec != codec {
        return Err(LuceneError::CorruptIndex(format!(
            "codec mismatch: actual codec={} vs expected codec={}",
            actual_codec, codec
        )));
    }

    let actual_version = read_be_int(input)?;
    if actual_version < min_version {
        return Err(LuceneError::IndexFormatNotSupported(format!(
            "index format too old: version={} minVersion={} maxVersion={}",
            actual_version, min_version, max_version
        )));
    }
    if actual_version > max_version {
        return Err(LuceneError::IndexFormatNotSupported(format!(
            "index format too new: version={} minVersion={} maxVersion={}",
            actual_version, min_version, max_version
        )));
    }

    Ok(actual_version)
}

/// Reads and validates an index header.
///
/// Equivalent to `CodecUtil.checkIndexHeader`.
pub fn check_index_header(
    input: &mut dyn DataInput,
    codec: &str,
    min_version: i32,
    max_version: i32,
    expected_id: &[u8],
    expected_suffix: &str,
) -> Result<i32> {
    let version = check_header(input, codec, min_version, max_version)?;
    check_index_header_id(input, expected_id)?;
    check_index_header_suffix(input, expected_suffix)?;
    Ok(version)
}

/// Reads and validates the 16-byte object id of an index header.
///
/// Equivalent to `CodecUtil.checkIndexHeaderID`.
pub fn check_index_header_id(input: &mut dyn DataInput, expected_id: &[u8]) -> Result<()> {
    if expected_id.len() != ID_LENGTH {
        return Err(LuceneError::IllegalArgument(format!(
            "expected id must be {} bytes, got {}",
            ID_LENGTH,
            expected_id.len()
        )));
    }
    let mut id = [0u8; ID_LENGTH];
    input.read_bytes(&mut id, 0, ID_LENGTH)?;
    if id != expected_id {
        return Err(LuceneError::CorruptIndex(format!(
            "file mismatch, expected id={:02x?}, got={:02x?}",
            expected_id, id
        )));
    }
    Ok(())
}

/// Reads and validates the suffix of an index header.
///
/// Equivalent to `CodecUtil.checkIndexHeaderSuffix`.
pub fn check_index_header_suffix(input: &mut dyn DataInput, expected_suffix: &str) -> Result<()> {
    let suffix_length = input.read_byte()? as usize;
    let mut suffix_bytes = vec![0u8; suffix_length];
    input.read_bytes(&mut suffix_bytes, 0, suffix_length)?;
    let suffix = String::from_utf8(suffix_bytes)
        .map_err(|e| LuceneError::IllegalArgument(format!("invalid UTF-8 reading suffix: {e}")))?;
    if suffix != expected_suffix {
        return Err(LuceneError::CorruptIndex(format!(
            "file mismatch, expected suffix={:?}, got={:?}",
            expected_suffix, suffix
        )));
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Footer writing and validation
// -----------------------------------------------------------------------------

/// Writes a codec footer.
///
/// Layout: `FOOTER_MAGIC` (big-endian int), algorithm id 0 (big-endian int),
/// CRC-32 of all preceding bytes (big-endian long).
///
/// Equivalent to `CodecUtil.writeFooter`.
pub fn write_footer(out: &mut dyn IndexOutput) -> Result<()> {
    write_be_int(out, FOOTER_MAGIC)?;
    write_be_int(out, 0)?;
    write_crc(out)?;
    Ok(())
}

/// Validates the footer and returns the computed checksum.
///
/// # Errors
///
/// Returns `LuceneError::CorruptIndex` if the footer is malformed or the
/// checksum does not match.
///
/// Equivalent to `CodecUtil.checkFooter(ChecksumIndexInput)`.
pub fn check_footer(input: &mut dyn ChecksumIndexInput) -> Result<i64> {
    validate_footer(input)?;
    let actual_checksum = input.get_checksum()?;
    let expected_checksum = read_crc(input)?;
    if expected_checksum != actual_checksum {
        return Err(LuceneError::CorruptIndex(format!(
            "checksum failed (hardware problem?) : expected={:x} actual={:x}",
            expected_checksum, actual_checksum
        )));
    }
    Ok(actual_checksum)
}

/// Validates the footer of an input whose entries already failed to decode.
///
/// Equivalent to `CodecUtil.checkFooter(ChecksumIndexInput, Throwable)`
/// (`CodecUtil.java:470-510`), which every codec's metadata reader uses inside
/// a `finally`: the entries are parsed first and the footer is verified
/// afterwards, whatever happened.
///
/// The decision Java makes is which of the two failures explains the file:
///
/// * the footer is unreachable, because the entry decoder already read into it
///   — the file is truncated or the entries claimed lengths that were not
///   there, and the reported failure says the checksum is indeterminate;
/// * the footer is corrupt — the file itself is damaged, so that corruption is
///   reported and `prior` is folded into the message;
/// * the checksum passes — the bytes are exactly what was written, so the
///   entries really are wrong and `prior` is reported unchanged.
///
/// Returning the error rather than a `Result` mirrors the fact that this is
/// only ever called when something has already gone wrong.
pub fn check_footer_with_prior(
    input: &mut dyn ChecksumIndexInput,
    prior: LuceneError,
) -> LuceneError {
    let remaining = input.length() - input.file_pointer();
    if remaining < footer_length() as i64 {
        return LuceneError::CorruptIndex(format!(
            "checksum status indeterminate: remaining={remaining}; please run \
             checkindex for more details (prior error: {prior})"
        ));
    }
    // Java skips the unread bytes rather than seeking, because the digest must
    // cover them; this input's `seek` is the forward-only, checksum-preserving
    // one for the same reason.
    let target = input.length() - footer_length() as i64;
    if let Err(error) = ChecksumIndexInput::seek(input, target) {
        return error;
    }
    match check_footer(input) {
        Ok(_) => prior,
        Err(corruption) => {
            LuceneError::CorruptIndex(format!("{corruption} (prior error: {prior})"))
        }
    }
}

/// Returns the checksum stored in the footer without validating it.
///
/// Equivalent to `CodecUtil.retrieveChecksum(IndexInput)`.
pub fn retrieve_checksum(input: &mut dyn IndexInput) -> Result<i64> {
    if input.length() < footer_length() as i64 {
        return Err(LuceneError::CorruptIndex(format!(
            "misplaced codec footer (file truncated?): length={} but footerLength={}",
            input.length(),
            footer_length()
        )));
    }
    input.seek(input.length() - footer_length() as i64)?;
    validate_footer(input)?;
    read_crc(input)
}

/// Returns the checksum stored in the footer, verifying the file length first.
///
/// Equivalent to `CodecUtil.retrieveChecksum(IndexInput, long)`.
pub fn retrieve_checksum_expected_length(
    input: &mut dyn IndexInput,
    expected_length: i64,
) -> Result<i64> {
    if expected_length < footer_length() as i64 {
        return Err(LuceneError::IllegalArgument(
            "expectedLength cannot be less than the footer length".to_string(),
        ));
    }
    if input.length() < expected_length {
        return Err(LuceneError::CorruptIndex(format!(
            "truncated file: length={} but expectedLength={}",
            input.length(),
            expected_length
        )));
    }
    if input.length() > expected_length {
        return Err(LuceneError::CorruptIndex(format!(
            "file too long: length={} but expectedLength={}",
            input.length(),
            expected_length
        )));
    }
    retrieve_checksum(input)
}

/// Clones `input`, reads all bytes, and validates the footer checksum.
///
/// Equivalent to `CodecUtil.checksumEntireFile`.
pub fn checksum_entire_file(input: &mut dyn IndexInput) -> Result<i64> {
    let mut clone = input.clone_input()?;
    clone.seek(0)?;
    let mut checksum_input = BufferedChecksumIndexInput::new(clone);
    if checksum_input.length() < footer_length() as i64 {
        return Err(LuceneError::CorruptIndex(format!(
            "misplaced codec footer (file truncated?): length={} but footerLength={}",
            checksum_input.length(),
            footer_length()
        )));
    }
    let target = checksum_input.length() - footer_length() as i64;
    ChecksumIndexInput::seek(&mut checksum_input, target)?;
    check_footer(&mut checksum_input)
}

fn validate_footer(input: &mut dyn IndexInput) -> Result<()> {
    let remaining = input.length() - input.file_pointer();
    let expected = footer_length() as i64;
    if remaining < expected {
        return Err(LuceneError::CorruptIndex(format!(
            "misplaced codec footer (file truncated?): remaining={}, expected={}, fp={}",
            remaining,
            expected,
            input.file_pointer()
        )));
    }
    if remaining > expected {
        return Err(LuceneError::CorruptIndex(format!(
            "misplaced codec footer (file extended?): remaining={}, expected={}, fp={}",
            remaining,
            expected,
            input.file_pointer()
        )));
    }

    let magic = read_be_int(input)?;
    if magic != FOOTER_MAGIC {
        return Err(LuceneError::CorruptIndex(format!(
            "codec footer mismatch (file truncated?): actual footer={} vs expected footer={}",
            magic, FOOTER_MAGIC
        )));
    }

    let algorithm_id = read_be_int(input)?;
    if algorithm_id != 0 {
        return Err(LuceneError::CorruptIndex(format!(
            "codec footer mismatch: unknown algorithmID: {}",
            algorithm_id
        )));
    }

    Ok(())
}

fn read_crc(input: &mut dyn DataInput) -> Result<i64> {
    let value = read_be_long(input)?;
    if (value as u64) > 0xFFFFFFFFu64 {
        return Err(LuceneError::CorruptIndex(format!(
            "Illegal CRC-32 checksum: {}",
            value
        )));
    }
    Ok(value)
}

fn write_crc(out: &mut dyn IndexOutput) -> Result<()> {
    let value = out.checksum()?;
    if (value as u64) > 0xFFFFFFFFu64 {
        return Err(LuceneError::IllegalState(format!(
            "Illegal CRC-32 checksum: {} (resource={})",
            value,
            out.resource_description()
        )));
    }
    write_be_long(out, value)
}

// -----------------------------------------------------------------------------
// Big-endian primitive helpers
// -----------------------------------------------------------------------------

/// Writes a big-endian `i32`.
///
/// Equivalent to `CodecUtil.writeBEInt`.
pub fn write_be_int(out: &mut dyn DataOutput, i: i32) -> Result<()> {
    out.write_byte((i >> 24) as u8)?;
    out.write_byte((i >> 16) as u8)?;
    out.write_byte((i >> 8) as u8)?;
    out.write_byte(i as u8)?;
    Ok(())
}

/// Writes a big-endian `i64`.
///
/// Equivalent to `CodecUtil.writeBELong`.
pub fn write_be_long(out: &mut dyn DataOutput, l: i64) -> Result<()> {
    write_be_int(out, (l >> 32) as i32)?;
    write_be_int(out, l as i32)?;
    Ok(())
}

/// Reads a big-endian `i32`.
///
/// Equivalent to `CodecUtil.readBEInt`.
pub fn read_be_int(input: &mut dyn DataInput) -> Result<i32> {
    let b1 = input.read_byte()? as i32;
    let b2 = input.read_byte()? as i32;
    let b3 = input.read_byte()? as i32;
    let b4 = input.read_byte()? as i32;
    Ok((b1 << 24) | (b2 << 16) | (b3 << 8) | b4)
}

/// Reads a big-endian `i64`.
///
/// Equivalent to `CodecUtil.readBELong`.
pub fn read_be_long(input: &mut dyn DataInput) -> Result<i64> {
    let high = read_be_int(input)? as i64;
    let low = read_be_int(input)? as i64;
    Ok((high << 32) | (low & 0xFFFFFFFFi64))
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{
        BufferedChecksumIndexInput, ByteArrayDataInput, MockIndexInput, MockIndexOutput,
    };

    // Minimal base64 decoder/encoder so the unit tests have no extra dependencies.
    mod base64 {
        pub fn decode(input: &str) -> Option<Vec<u8>> {
            let mut output = Vec::with_capacity(input.len() * 3 / 4);
            let mut buf = 0u32;
            let mut bits = 0u32;
            let mut padding_seen = false;

            for ch in input.bytes() {
                if ch == b'=' {
                    padding_seen = true;
                    continue;
                }
                if padding_seen {
                    return None;
                }
                let value = match ch {
                    b'A'..=b'Z' => ch - b'A',
                    b'a'..=b'z' => ch - b'a' + 26,
                    b'0'..=b'9' => ch - b'0' + 52,
                    b'+' => 62,
                    b'/' => 63,
                    _ => return None,
                } as u32;
                buf = (buf << 6) | value;
                bits += 6;
                if bits >= 8 {
                    bits -= 8;
                    output.push(((buf >> bits) & 0xFF) as u8);
                }
            }
            Some(output)
        }

        pub fn encode(input: &[u8]) -> String {
            const ALPHABET: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
            let mut chunks = input.chunks_exact(3);
            for chunk in chunks.by_ref() {
                let n = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32);
                output.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
                output.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
                output.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
                output.push(ALPHABET[(n & 0x3F) as usize] as char);
            }
            let rem = chunks.remainder();
            match rem.len() {
                0 => {}
                1 => {
                    let n = (rem[0] as u32) << 16;
                    output.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
                    output.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
                    output.push('=');
                    output.push('=');
                }
                2 => {
                    let n = ((rem[0] as u32) << 16) | ((rem[1] as u32) << 8);
                    output.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
                    output.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
                    output.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
                    output.push('=');
                }
                _ => unreachable!(),
            }
            output
        }

        #[test]
        fn round_trip() {
            assert_eq!(decode(""), Some(vec![]));
            assert_eq!(decode("QQ=="), Some(vec![b'A']));
            assert_eq!(decode("QUI="), Some(vec![b'A', b'B']));
            assert_eq!(decode("QUJD"), Some(vec![b'A', b'B', b'C']));
            assert_eq!(encode(b"Man"), "TWFu");
            assert_eq!(encode(b"M"), "TQ==");
            assert_eq!(encode(b"Ma"), "TWE=");
        }
    }

    #[test]
    fn constants_match_lucene() {
        assert_eq!(CODEC_MAGIC, 0x3fd76c17);
        assert_eq!(FOOTER_MAGIC, !0x3fd76c17);
        assert_eq!(footer_length(), 16);
    }

    #[test]
    fn write_and_check_header_round_trip() {
        let mut out = MockIndexOutput::new("test", "test.bin");
        write_header(&mut out, "TestCodec", 5).unwrap();
        let bytes = out.into_inner();

        let mut input = ByteArrayDataInput::new(bytes);
        let version = check_header(&mut input, "TestCodec", 4, 6).unwrap();
        assert_eq!(version, 5);
    }

    #[test]
    fn header_rejects_bad_magic() {
        let mut bytes = vec![0u8; 4];
        bytes.copy_from_slice(&0i32.to_be_bytes());
        let mut input = ByteArrayDataInput::new(bytes);
        let err = check_header(&mut input, "TestCodec", 1, 1).unwrap_err();
        assert!(matches!(err, LuceneError::CorruptIndex(_)));
        assert!(err.to_string().contains("codec header mismatch"));
    }

    #[test]
    fn header_rejects_wrong_codec() {
        let mut out = MockIndexOutput::new("test", "test.bin");
        write_header(&mut out, "RightCodec", 5).unwrap();
        let bytes = out.into_inner();

        let mut input = ByteArrayDataInput::new(bytes);
        let err = check_header(&mut input, "WrongCodec", 1, 10).unwrap_err();
        assert!(matches!(err, LuceneError::CorruptIndex(_)));
        assert!(err.to_string().contains("codec mismatch"));
    }

    #[test]
    fn header_rejects_version_out_of_range() {
        let mut out = MockIndexOutput::new("test", "test.bin");
        write_header(&mut out, "TestCodec", 5).unwrap();
        let bytes = out.into_inner();

        let mut input = ByteArrayDataInput::new(bytes.clone());
        let err = check_header(&mut input, "TestCodec", 6, 10).unwrap_err();
        assert!(matches!(err, LuceneError::IndexFormatNotSupported(_)));
        assert!(err.to_string().contains("too old"));

        let mut input = ByteArrayDataInput::new(bytes);
        let err = check_header(&mut input, "TestCodec", 1, 4).unwrap_err();
        assert!(matches!(err, LuceneError::IndexFormatNotSupported(_)));
        assert!(err.to_string().contains("too new"));
    }

    #[test]
    fn write_and_check_footer_round_trip() {
        let mut out = MockIndexOutput::new("test", "test.bin");
        write_header(&mut out, "FootCodec", 1).unwrap();
        out.write_v_int(12345).unwrap();
        out.write_string("payload").unwrap();
        write_footer(&mut out).unwrap();
        let bytes = out.into_inner();

        // CRC-32 of the whole file except the trailing long.
        let mut expected = crate::store::BufferedChecksum::new();
        expected.update_bytes(&bytes, 0, bytes.len() - 8).unwrap();

        let input = MockIndexInput::new(bytes, "test.bin");
        let mut checksum_input = BufferedChecksumIndexInput::new(Box::new(input));
        // Skip the payload so the footer is next.
        let data_end = checksum_input.length() - footer_length() as i64;
        ChecksumIndexInput::seek(&mut checksum_input, data_end).unwrap();
        let actual = check_footer(&mut checksum_input).unwrap();

        assert_eq!(actual, expected.get_value());
    }

    #[test]
    fn footer_rejects_bad_magic() {
        let mut out = MockIndexOutput::new("test", "test.bin");
        write_header(&mut out, "FootCodec", 1).unwrap();
        write_footer(&mut out).unwrap();
        let mut bytes = out.into_inner();

        // Corrupt the footer magic (last 16 bytes: 4 magic + 4 alg + 8 crc).
        let footer_start = bytes.len() - 16;
        bytes[footer_start] = 0x00;

        let input = MockIndexInput::new(bytes, "test.bin");
        let mut checksum_input = BufferedChecksumIndexInput::new(Box::new(input));
        let footer_start = checksum_input.length() - footer_length() as i64;
        ChecksumIndexInput::seek(&mut checksum_input, footer_start).unwrap();
        let err = check_footer(&mut checksum_input).unwrap_err();
        assert!(matches!(err, LuceneError::CorruptIndex(_)));
        assert!(err.to_string().contains("codec footer mismatch"));
    }

    #[test]
    fn footer_rejects_bad_checksum() {
        let mut out = MockIndexOutput::new("test", "test.bin");
        write_header(&mut out, "FootCodec", 1).unwrap();
        write_footer(&mut out).unwrap();
        let mut bytes = out.into_inner();

        // Corrupt the CRC (last 8 bytes).
        let crc_start = bytes.len() - 8;
        bytes[crc_start] = bytes[crc_start].wrapping_add(1);

        let input = MockIndexInput::new(bytes, "test.bin");
        let mut checksum_input = BufferedChecksumIndexInput::new(Box::new(input));
        let footer_start = checksum_input.length() - footer_length() as i64;
        ChecksumIndexInput::seek(&mut checksum_input, footer_start).unwrap();
        let err = check_footer(&mut checksum_input).unwrap_err();
        assert!(matches!(err, LuceneError::CorruptIndex(_)));
    }

    #[test]
    fn retrieve_checksum_matches_java_fixture() {
        let bytes =
            base64::decode("P9dsFw9SdWNlbmVDb2RlY1V0aWwAAAAquWAHcGF5bG9hZMAok+gAAAAAAAAAAKxoMEk=")
                .expect("embedded base64 is valid");
        let mut input = MockIndexInput::new(bytes, "codecutil.bin");
        let crc = retrieve_checksum(&mut input).unwrap();
        assert_eq!(crc, 0xac683049i64);
    }

    #[test]
    fn checksum_entire_file_matches_java_fixture() {
        let bytes =
            base64::decode("P9dsFw9SdWNlbmVDb2RlY1V0aWwAAAAquWAHcGF5bG9hZMAok+gAAAAAAAAAAKxoMEk=")
                .expect("embedded base64 is valid");
        let mut input = MockIndexInput::new(bytes.clone(), "codecutil.bin");
        let crc = checksum_entire_file(&mut input).unwrap();
        assert_eq!(crc, 0xac683049i64);
    }

    #[test]
    fn index_header_round_trip() {
        let mut out = MockIndexOutput::new("test", "idx.bin");
        let id: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        write_index_header(&mut out, "RuceneIdx", 7, &id, "_0").unwrap();
        out.write_byte(0x99).unwrap();
        write_footer(&mut out).unwrap();
        let bytes = out.into_inner();

        let expected_b64 =
            "P9dsFwlSdWNlbmVJZHgAAAAHAAECAwQFBgcICQoLDA0ODwJfMJnAKJPoAAAAAAAAAAA7uEXh";
        assert_eq!(base64::encode(&bytes), expected_b64);

        let mut input = ByteArrayDataInput::new(bytes);
        let version = check_index_header(&mut input, "RuceneIdx", 7, 7, &id, "_0").unwrap();
        assert_eq!(version, 7);
        assert_eq!(input.read_byte().unwrap(), 0x99);
    }

    #[test]
    fn big_endian_helpers_round_trip() {
        let mut out = MockIndexOutput::new("test", "be.bin");
        write_be_int(&mut out, 0x12345678).unwrap();
        write_be_long(&mut out, 0x123456789ABCDEF0i64).unwrap();
        let bytes = out.into_inner();

        let mut input = ByteArrayDataInput::new(bytes);
        assert_eq!(read_be_int(&mut input).unwrap(), 0x12345678);
        assert_eq!(read_be_long(&mut input).unwrap(), 0x123456789ABCDEF0i64);
    }

    #[test]
    fn codec_name_validation() {
        let mut out = MockIndexOutput::new("test", "test.bin");
        let err = write_header(&mut out, "Bad\u{00E9}Codec", 1).unwrap_err();
        assert!(matches!(err, LuceneError::IllegalArgument(_)));

        let mut out = MockIndexOutput::new("test", "test.bin");
        let long = "a".repeat(128);
        let err = write_header(&mut out, &long, 1).unwrap_err();
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }
}
