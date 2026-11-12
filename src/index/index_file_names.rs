//! File-name constants and helpers ported from `org.apache.lucene.index.IndexFileNames`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, LuceneErrorKind, Result};

/// Name of the index segment file.
///
/// Equivalent to `IndexFileNames.SEGMENTS`.
pub const SEGMENTS: &str = "segments";

/// Name of the pending index segment file.
///
/// Equivalent to `IndexFileNames.PENDING_SEGMENTS`.
pub const PENDING_SEGMENTS: &str = "pending_segments";

/// Segment-info file extension.
pub const SEGMENT_INFO_EXTENSION: &str = "si";

/// Field-infos file extension.
pub const FIELD_INFO_EXTENSION: &str = "fnm";

/// Stored-fields data file extension.
pub const STORED_FIELDS_EXTENSION: &str = "fdt";

/// Stored-fields index file extension.
pub const STORED_FIELDS_INDEX_EXTENSION: &str = "fdx";

/// Stored-fields metadata file extension.
pub const STORED_FIELDS_META_EXTENSION: &str = "fdm";

/// Term-vectors data file extension.
pub const VECTORS_FIELDS_EXTENSION: &str = "tvd";

/// Term-vectors index file extension.
pub const VECTORS_INDEX_EXTENSION: &str = "tvx";

/// Term-vectors metadata file extension.
pub const VECTORS_META_EXTENSION: &str = "tvm";

/// Postings primary file extension.
pub const POSTINGS_EXTENSION: &str = "doc";

/// Postings positions file extension.
pub const POSITIONS_EXTENSION: &str = "pos";

/// Postings payloads file extension.
pub const PAYLOADS_EXTENSION: &str = "pay";

/// Postings skip/impacts file extension.
pub const TERMS_POSTINGS_EXTENSION: &str = "psm";

/// Terms dictionary file extension.
pub const TERMS_EXTENSION: &str = "tim";

/// Terms index file extension.
pub const TERMS_INDEX_EXTENSION: &str = "tip";

/// Terms metadata file extension.
pub const TERMS_META_EXTENSION: &str = "tmd";

/// Doc-values data file extension.
pub const DOC_VALUES_EXTENSION: &str = "dvd";

/// Doc-values metadata file extension.
pub const DOC_VALUES_META_EXTENSION: &str = "dvm";

/// Norms data file extension.
pub const NORMS_EXTENSION: &str = "nvd";

/// Norms metadata file extension.
pub const NORMS_META_EXTENSION: &str = "nvm";

/// Points data file extension.
pub const POINTS_EXTENSION: &str = "kdd";

/// Points index file extension.
pub const POINTS_INDEX_EXTENSION: &str = "kdi";

/// Points metadata file extension.
pub const POINTS_META_EXTENSION: &str = "kdm";

/// KNN vectors data file extension.
pub const KNN_VECTORS_EXTENSION: &str = "vec";

/// KNN vectors index file extension.
pub const KNN_VECTORS_INDEX_EXTENSION: &str = "vex";

/// KNN vectors metadata file extension.
pub const KNN_VECTORS_META_EXTENSION: &str = "vem";

/// KNN flat vectors metadata file extension.
pub const KNN_VECTORS_FORMAT_META_EXTENSION: &str = "vemf";

/// Live-docs bitset file extension.
pub const LIVE_DOCS_EXTENSION: &str = "liv";

/// Compound-file data file extension.
pub const COMPOUND_FILE_EXTENSION: &str = "cfs";

/// Compound-file entry-table extension.
pub const COMPOUND_FILE_ENTRIES_EXTENSION: &str = "cfe";

/// Old, pre-Lucene-8.6 live-docs extension.
pub const OLD_LIVE_DOCS_EXTENSION: &str = "del";

/// Computes the full file name from base, extension and generation.
///
/// Returns `None` if `gen` is `-1`. Returns `<base>.<ext>` for `gen == 0`, and
/// `<base>_<gen>.<ext>` for `gen > 0`.
///
/// Equivalent to `IndexFileNames.fileNameFromGeneration`.
pub fn file_name_from_generation(base: &str, ext: &str, gen: i64) -> Option<String> {
    if gen == -1 {
        None
    } else if gen == 0 {
        Some(segment_file_name(base, "", ext))
    } else {
        debug_assert!(gen > 0);
        let mut res = String::with_capacity(base.len() + 6 + ext.len());
        res.push_str(base);
        res.push('_');
        res.push_str(&radix36(gen as u64));
        if !ext.is_empty() {
            res.push('.');
            res.push_str(ext);
        }
        Some(res)
    }
}

/// Builds a file name from segment name, suffix and extension.
///
/// Format: `<segmentName>(_<segmentSuffix>)(.<ext>)`.
///
/// Equivalent to `IndexFileNames.segmentFileName`.
pub fn segment_file_name(segment_name: &str, segment_suffix: &str, ext: &str) -> String {
    if ext.is_empty() && segment_suffix.is_empty() {
        segment_name.to_string()
    } else {
        debug_assert!(!ext.starts_with('.'));
        let mut sb =
            String::with_capacity(segment_name.len() + 2 + segment_suffix.len() + ext.len());
        sb.push_str(segment_name);
        if !segment_suffix.is_empty() {
            sb.push('_');
            sb.push_str(segment_suffix);
        }
        if !ext.is_empty() {
            sb.push('.');
            sb.push_str(ext);
        }
        sb
    }
}

/// Returns `true` if `filename` ends with `.ext`.
///
/// Equivalent to `IndexFileNames.matchesExtension`.
pub fn matches_extension(filename: &str, ext: &str) -> bool {
    filename.ends_with(&format!(".{ext}"))
}

/// Locates the boundary of the segment name in `filename`, or `None`.
fn index_of_segment_name(filename: &str) -> Option<usize> {
    // If it is a .del file, there's an '_' after the first character.
    //
    // Java writes `filename.indexOf('_', 1)` (`IndexFileNames.java:121`),
    // which starts the search one *UTF-16 code unit* in, and so never matches a
    // leading underscore. Slicing `filename[1..]` would instead skip one *byte*
    // and panic whenever the name begins with a multi-byte UTF-8 character —
    // and these names are not always well-formed, because `IndexFileDeleter`
    // feeds this function the directory's pending-deletion set, which is
    // unfiltered (`IndexFileDeleter.java:211-217`). Starting one *character* in
    // reaches the same underscore as Java for every input, because the units a
    // multi-byte character occupies can never themselves be `'_'`, and it never
    // panics.
    let after_first = {
        let mut chars = filename.chars();
        chars.next();
        chars.as_str()
    };
    let offset = filename.len() - after_first.len();
    if let Some(idx) = after_first.find('_') {
        return Some(idx + offset);
    }
    filename.find('.')
}

/// Strips the segment name from `filename`.
///
/// Equivalent to `IndexFileNames.stripSegmentName`.
pub fn strip_segment_name(filename: &str) -> &str {
    if let Some(idx) = index_of_segment_name(filename) {
        &filename[idx..]
    } else {
        filename
    }
}

/// Parses the segment name out of `filename`.
///
/// Equivalent to `IndexFileNames.parseSegmentName`.
pub fn parse_segment_name(filename: &str) -> &str {
    if let Some(idx) = index_of_segment_name(filename) {
        &filename[..idx]
    } else {
        filename
    }
}

/// Removes the extension (anything after the first '.') from `filename`.
///
/// Equivalent to `IndexFileNames.stripExtension`.
pub fn strip_extension(filename: &str) -> &str {
    if let Some(idx) = filename.find('.') {
        &filename[..idx]
    } else {
        filename
    }
}

/// Returns the extension (anything after the first '.'), or `None`.
///
/// Equivalent to `IndexFileNames.getExtension`.
pub fn get_extension(filename: &str) -> Option<&str> {
    filename.find('.').map(|idx| &filename[idx + 1..])
}

/// Returns the generation encoded in `filename`, or `0` if absent.
///
/// Equivalent to `IndexFileNames.parseGeneration` (`IndexFileNames.java:146-160`),
/// which only asserts that `filename` starts with `'_'`.
///
/// # Errors
///
/// Returns Java's `StringIndexOutOfBoundsException` when the name without its
/// extension is empty (`substring(1)`), and `LuceneErrorKind::IllegalArgument`
/// (Java's `NumberFormatException`) when the generation is not base 36.
pub fn parse_generation(filename: &str) -> Result<i64> {
    debug_assert!(filename.starts_with('_'));
    let stripped = strip_extension(filename);
    // `stripExtension(filename).substring(1)`.
    let mut chars = stripped.chars();
    if chars.next().is_none() {
        return Err(LuceneError::from(LuceneErrorKind::StringIndexOutOfBounds(
            "Range [1, 0) out of bounds for length 0".to_string(),
        )));
    }
    let rest = chars.as_str();
    // `String.split("_")`: without a match, the whole string; otherwise the
    // pieces without the trailing empty ones.
    let mut parts: Vec<&str> = rest.split('_').collect();
    if parts.len() > 1 {
        while parts.last() == Some(&"") {
            parts.pop();
        }
    }
    // 4 cases:
    // segment.ext
    // segment_gen.ext
    // segment_codec_suffix.ext
    // segment_gen_codec_suffix.ext
    if parts.len() == 2 || parts.len() == 4 {
        parse_radix36(parts[1])
    } else {
        Ok(0)
    }
}

/// The regular expression every codec file name must match.
///
/// Equivalent to the pattern string of `IndexFileNames.CODEC_FILE_PATTERN`,
/// `_[a-z0-9]+(_.*)?\..*`. The port matches it by hand in [`is_codec_file`]
/// rather than by compiling a regex; the literal is kept so that the messages
/// Lucene builds from `CODEC_FILE_PATTERN.pattern()` read identically.
pub const CODEC_FILE_PATTERN: &str = r"_[a-z0-9]+(_.*)?\..*";

/// Returns whether `filename` matches the codec file-name pattern.
///
/// Equivalent to `IndexFileNames.CODEC_FILE_PATTERN`.
pub fn is_codec_file(filename: &str) -> bool {
    // Pattern: _[a-z0-9]+(_.*)?\..*
    let bytes = filename.as_bytes();
    if bytes.is_empty() || bytes[0] != b'_' {
        return false;
    }
    let mut i = 1;
    // At least one lowercase alphanumeric.
    if i >= bytes.len() || !bytes[i].is_ascii_lowercase() && !bytes[i].is_ascii_digit() {
        return false;
    }
    while i < bytes.len() && (bytes[i].is_ascii_lowercase() || bytes[i].is_ascii_digit()) {
        i += 1;
    }
    // Optional suffix starting with '_'.
    if i < bytes.len() && bytes[i] == b'_' {
        i += 1;
        while i < bytes.len() && bytes[i] != b'.' {
            i += 1;
        }
    }
    // Must contain a '.' and at least one character after it.
    i < bytes.len() && bytes[i] == b'.' && i + 1 < bytes.len()
}

/// Formats `value` in base-36, matching Java's `Long.toString(gen, Character.MAX_RADIX)`.
fn radix36(value: u64) -> String {
    if value == 0 {
        "0".to_string()
    } else {
        let mut digits = Vec::new();
        let mut v = value;
        while v > 0 {
            let digit = (v % 36) as u8;
            let c = if digit < 10 {
                b'0' + digit
            } else {
                b'a' + (digit - 10)
            };
            digits.push(c);
            v /= 36;
        }
        digits.reverse();
        String::from_utf8(digits).unwrap()
    }
}

/// `Long.parseLong(s, Character.MAX_RADIX)`, failing with its
/// `NumberFormatException` (`IndexFileNames.java:155`).
fn parse_radix36(s: &str) -> Result<i64> {
    crate::util::extra::java_parse_long_radix(s, 36)
        .ok_or_else(|| LuceneError::number_format_for_input_string(s, 36))
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_name_from_generation_matches_java() {
        assert_eq!(file_name_from_generation("_0", "si", -1), None);
        assert_eq!(
            file_name_from_generation("_0", "si", 0),
            Some("_0.si".to_string())
        );
        assert_eq!(
            file_name_from_generation("_0", "si", 1),
            Some("_0_1.si".to_string())
        );
        assert_eq!(
            file_name_from_generation("_0", "si", 35),
            Some("_0_z.si".to_string())
        );
        assert_eq!(
            file_name_from_generation("_0", "si", 36),
            Some("_0_10.si".to_string())
        );
    }

    #[test]
    fn segment_file_name_builds_variants() {
        assert_eq!(segment_file_name("_0", "", ""), "_0");
        assert_eq!(segment_file_name("_0", "", "si"), "_0.si");
        assert_eq!(segment_file_name("_0", "x", "si"), "_0_x.si");
    }

    #[test]
    fn matches_extension_checks_suffix() {
        assert!(matches_extension("_0.si", "si"));
        assert!(!matches_extension("_0.si", "fnm"));
        assert!(!matches_extension("_0si", "si"));
    }

    #[test]
    fn parse_segment_name_and_generation() {
        assert_eq!(parse_segment_name("_0.si"), "_0");
        assert_eq!(parse_segment_name("_0_1.si"), "_0");
        assert_eq!(parse_segment_name("_0_x_foo.si"), "_0");
        assert_eq!(parse_segment_name("segments"), "segments");

        assert_eq!(parse_generation("_0.si").unwrap(), 0);
        assert_eq!(parse_generation("_0_1.si").unwrap(), 1);
        assert_eq!(parse_generation("_0_z.si").unwrap(), 35);
        assert_eq!(parse_generation("_0_1_x.si").unwrap(), 0);
    }

    #[test]
    fn strip_and_get_extension() {
        assert_eq!(strip_extension("_0.si"), "_0");
        assert_eq!(get_extension("_0.si"), Some("si"));
        assert_eq!(get_extension("_0"), None);
    }

    #[test]
    fn codec_file_pattern() {
        assert!(is_codec_file("_0.si"));
        assert!(is_codec_file("_0_1.fnm"));
        assert!(is_codec_file("_0_x_foo.doc"));
        assert!(!is_codec_file("segments"));
        assert!(!is_codec_file("_0"));
    }

    /// Java only asserts the leading `'_'` (`IndexFileNames.java:147`): with
    /// assertions enabled (debug builds) the assert fails; without, the first
    /// character is dropped whatever it is.
    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "filename.starts_with('_')"))]
    fn parse_generation_asserts_the_leading_underscore() {
        assert_eq!(parse_generation("0_1.si").unwrap(), 1);
    }

    /// `String.split` drops trailing empty strings, and `substring(1)` of an
    /// empty name throws.
    #[test]
    fn parse_generation_splits_as_java() {
        assert_eq!(parse_generation("_0_.si").unwrap(), 0);
        assert_eq!(parse_generation("_0__.si").unwrap(), 0);
        assert_eq!(parse_generation("_0_1_.si").unwrap(), 1);
        assert!(matches!(parse_generation("_.si"), Ok(0)));
    }
}
