// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.CollationDataReader`.

use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::ByteBuffer;
use crate::r#impl::coll::collation::Collation;
use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::coll::collation_fast_latin::CollationFastLatin;
use crate::r#impl::coll::collation_keys::CollationKeys;
use crate::r#impl::coll::collation_root_elements::CollationRootElements;
use crate::r#impl::coll::collation_tailoring::CollationTailoring;
use crate::r#impl::icu_binary::ICUBinary;
use crate::r#impl::trie2_32::Trie2_32;
use crate::r#impl::u_serialized_set::USerializedSet;
use crate::text::collator::ReorderCodes;
use crate::text::unicode_set::UnicodeSet;
use crate::util::icu_exception::ICUException;

/// Collation binary data reader.
///
/// Equivalent to `com.ibm.icu.impl.coll.CollationDataReader` (package
/// private, static).
#[derive(Debug)]
pub struct CollationDataReader;

fn icu(message: &str) -> rucene::error::LuceneError {
    ICUException::with_message(message).error()
}

const DATA_FORMAT: i32 = 0x55436f6c; // "UCol"

/// Whether the data file's format version is supported.
///
/// Equivalent to the private class `com.ibm.icu.impl.coll.CollationDataReader.IsAcceptable`,
/// whose `IsAcceptable.isDataVersionAcceptable(byte[])` is this function
/// (an `ICUBinary.Authenticate`).
fn is_acceptable(version: &[u8; 4]) -> bool {
    version[0] == 5
}

impl CollationDataReader {
    /// Number of int indexes.
    pub const IX_INDEXES_LENGTH: usize = 0;
    /// Bits 31..24: numericPrimary, for numeric collation; 23..16: fast
    /// Latin format version (0 = no fast Latin table); 15.. 0: options bit
    /// set.
    pub const IX_OPTIONS: usize = 1;
    /// `IX_RESERVED2`.
    pub const IX_RESERVED2: usize = 2;
    /// `IX_RESERVED3`.
    pub const IX_RESERVED3: usize = 3;
    /// Array offset to Jamo CE32s in ce32s[], or <0 if none.
    pub const IX_JAMO_CE32S_START: usize = 4;
    /// Byte offset to int reorderCodes[].
    pub const IX_REORDER_CODES_OFFSET: usize = 5;
    /// Byte offset to uint8_t reorderTable[].
    pub const IX_REORDER_TABLE_OFFSET: usize = 6;
    /// Byte offset to the collation trie. Its length is a multiple of 8 bytes.
    pub const IX_TRIE_OFFSET: usize = 7;
    /// `IX_RESERVED8_OFFSET`.
    pub const IX_RESERVED8_OFFSET: usize = 8;
    /// Byte offset to long ces[].
    pub const IX_CES_OFFSET: usize = 9;
    /// `IX_RESERVED10_OFFSET`.
    pub const IX_RESERVED10_OFFSET: usize = 10;
    /// Byte offset to int ce32s[].
    pub const IX_CE32S_OFFSET: usize = 11;
    /// Byte offset to uint32_t rootElements[].
    pub const IX_ROOT_ELEMENTS_OFFSET: usize = 12;
    /// Byte offset to UChar *contexts[].
    pub const IX_CONTEXTS_OFFSET: usize = 13;
    /// Byte offset to char [] with serialized unsafeBackwardSet.
    pub const IX_UNSAFE_BWD_OFFSET: usize = 14;
    /// Byte offset to char fastLatinTable[].
    pub const IX_FAST_LATIN_TABLE_OFFSET: usize = 15;
    /// Byte offset to char scripts[].
    pub const IX_SCRIPTS_OFFSET: usize = 16;
    /// Byte offset to boolean compressibleBytes[].
    pub const IX_COMPRESSIBLE_BYTES_OFFSET: usize = 17;
    /// `IX_RESERVED18_OFFSET`.
    pub const IX_RESERVED18_OFFSET: usize = 18;
    /// `IX_TOTAL_SIZE`.
    pub const IX_TOTAL_SIZE: usize = 19;

    /// `read(CollationTailoring, ByteBuffer, CollationTailoring)`.
    ///
    /// # Errors
    ///
    /// `ICUException` for invalid data, `IOException` of the header and
    /// buffer reads.
    pub fn read(
        base: Option<&CollationTailoring>,
        in_bytes: &mut ByteBuffer,
        tailoring: &mut CollationTailoring,
    ) -> Result<()> {
        tailoring.version = ICUBinary::read_header(
            in_bytes,
            DATA_FORMAT,
            Some(&is_acceptable as &dyn Fn(&[u8; 4]) -> bool),
        )?;
        if let Some(base) = base {
            if base.get_uca_version() != tailoring.get_uca_version() {
                return Err(icu(
                    "Tailoring UCA version differs from base data UCA version",
                ));
            }
        }

        let in_length = in_bytes.remaining();
        if in_length < 8 {
            return Err(icu("not enough bytes"));
        }
        let indexes_length = in_bytes.get_int()?; // inIndexes[IX_INDEXES_LENGTH]
        if indexes_length < 2 || in_length < indexes_length * 4 {
            return Err(icu("not enough indexes"));
        }
        let mut in_indexes = [0i32; Self::IX_TOTAL_SIZE + 1];
        in_indexes[0] = indexes_length;
        let mut i = 1;
        while (i as i32) < indexes_length && i < in_indexes.len() {
            in_indexes[i] = in_bytes.get_int()?;
            i += 1;
        }
        for v in in_indexes.iter_mut().skip(indexes_length.max(0) as usize) {
            *v = -1;
        }
        if indexes_length > in_indexes.len() as i32 {
            ICUBinary::skip_bytes(in_bytes, (indexes_length - in_indexes.len() as i32) * 4)?;
        }

        // Assume that the tailoring data is in initial state,
        // with null pointers and 0 lengths.

        // Set pointers to non-empty data parts.
        // Do this in order of their byte offsets. (Should help porting to Java.)

        let mut index: usize; // one of the indexes[] slots
        let mut offset: i32; // byte offset for the index part
        let mut length: i32; // number of bytes in the index part

        if indexes_length > Self::IX_TOTAL_SIZE as i32 {
            length = in_indexes[Self::IX_TOTAL_SIZE];
        } else if indexes_length > Self::IX_REORDER_CODES_OFFSET as i32 {
            length = in_indexes[(indexes_length - 1) as usize];
        } else {
            length = 0; // only indexes, and inLength was already checked for them
        }
        if in_length < length {
            return Err(icu("not enough bytes"));
        }

        let base_data: Option<Arc<CollationData>> = base.map(|b| b.data().clone());

        let reorder_codes: Vec<i32>;
        let mut reorder_codes_length: i32;
        index = Self::IX_REORDER_CODES_OFFSET;
        offset = in_indexes[index];
        length = in_indexes[index + 1] - offset;
        if length >= 4 {
            if base_data.is_none() {
                // We assume for collation settings that
                // the base data does not have a reordering.
                return Err(icu("Collation base data must not reorder scripts"));
            }
            reorder_codes_length = length / 4;
            reorder_codes = ICUBinary::get_ints(in_bytes, reorder_codes_length, length & 3)?;

            // The reorderRanges (if any) are the trailing reorderCodes entries.
            // Split the array at the boundary.
            // Script or reorder codes do not exceed 16-bit values.
            // Range limits are stored in the upper 16 bits, and are never 0.
            let mut reorder_ranges_length = 0;
            while reorder_ranges_length < reorder_codes_length
                && (reorder_codes[(reorder_codes_length - reorder_ranges_length - 1) as usize]
                    as u32
                    & 0xffff0000)
                    != 0
            {
                reorder_ranges_length += 1;
            }
            debug_assert!(reorder_ranges_length < reorder_codes_length);
            reorder_codes_length -= reorder_ranges_length;
        } else {
            reorder_codes = Vec::new();
            reorder_codes_length = 0;
            ICUBinary::skip_bytes(in_bytes, length)?;
        }

        // There should be a reorder table only if there are reorder codes.
        // However, when there are reorder codes the reorder table may be omitted to reduce
        // the data size.
        let mut reorder_table: Option<Arc<[i8]>> = None;
        index = Self::IX_REORDER_TABLE_OFFSET;
        offset = in_indexes[index];
        length = in_indexes[index + 1] - offset;
        if length >= 256 {
            if reorder_codes_length == 0 {
                return Err(icu("Reordering table without reordering codes"));
            }
            let mut table = [0u8; 256];
            in_bytes.get_bytes(&mut table)?;
            reorder_table = Some(table.iter().map(|&b| b as i8).collect());
            length -= 256;
        } else {
            // If we have reorder codes, then build the reorderTable at the end,
            // when the CollationData is otherwise complete.
        }
        ICUBinary::skip_bytes(in_bytes, length)?;

        if let Some(base_data) = &base_data {
            if base_data.numeric_primary != ((in_indexes[Self::IX_OPTIONS] as i64) & 0xff000000) {
                return Err(icu(
                    "Tailoring numeric primary weight differs from base data",
                ));
            }
        }
        let mut data: Option<CollationData> = None; // Remains null if there are no mappings.

        index = Self::IX_TRIE_OFFSET;
        offset = in_indexes[index];
        length = in_indexes[index + 1] - offset;
        if length >= 8 {
            let mut d = tailoring.new_owned_data()?;
            d.base = base_data.clone();
            d.numeric_primary = (in_indexes[Self::IX_OPTIONS] as i64) & 0xff000000;
            let trie = Arc::new(Trie2_32::create_from_serialized(in_bytes)?);
            d.trie = Some(trie.clone());
            tailoring.trie = Some(trie.clone());
            let trie_length = trie.get_serialized_length();
            if trie_length > length {
                return Err(icu("Not enough bytes for the mappings trie")); // No mappings.
            }
            length -= trie_length;
            data = Some(d);
        } else if let Some(base_data) = &base_data {
            // Use the base data. Only the settings are tailored.
            tailoring.data = Some(base_data.clone());
        } else {
            return Err(icu("Missing collation data mappings")); // No mappings.
        }
        ICUBinary::skip_bytes(in_bytes, length)?;

        index = Self::IX_RESERVED8_OFFSET;
        offset = in_indexes[index];
        length = in_indexes[index + 1] - offset;
        ICUBinary::skip_bytes(in_bytes, length)?;

        index = Self::IX_CES_OFFSET;
        offset = in_indexes[index];
        length = in_indexes[index + 1] - offset;
        if length >= 8 {
            let Some(d) = data.as_mut() else {
                return Err(icu("Tailored ces without tailored trie"));
            };
            d.ces = Arc::from(ICUBinary::get_longs(in_bytes, length / 8, length & 7)?);
        } else {
            ICUBinary::skip_bytes(in_bytes, length)?;
        }

        index = Self::IX_RESERVED10_OFFSET;
        offset = in_indexes[index];
        length = in_indexes[index + 1] - offset;
        ICUBinary::skip_bytes(in_bytes, length)?;

        // Java's data.ce32s stays null until read; an array read here is never empty.
        let mut ce32s_read = false;
        index = Self::IX_CE32S_OFFSET;
        offset = in_indexes[index];
        length = in_indexes[index + 1] - offset;
        if length >= 4 {
            let Some(d) = data.as_mut() else {
                return Err(icu("Tailored ce32s without tailored trie"));
            };
            d.ce32s = Arc::from(ICUBinary::get_ints(in_bytes, length / 4, length & 3)?);
            ce32s_read = true;
        } else {
            ICUBinary::skip_bytes(in_bytes, length)?;
        }

        let jamo_ce32s_start = in_indexes[Self::IX_JAMO_CE32S_START];
        if jamo_ce32s_start >= 0 {
            match data.as_mut() {
                Some(d) if ce32s_read => {
                    let start = jamo_ce32s_start as usize;
                    let end = start + CollationData::JAMO_CE32S_LENGTH as usize;
                    if end > d.ce32s.len() {
                        crate::jdk::array_index_out_of_bounds(d.ce32s.len() as i32);
                    }
                    d.jamo_ce32s = Arc::from(&d.ce32s[start..end]);
                }
                _ => return Err(icu("JamoCE32sStart index into non-existent ce32s[]")),
            }
        } else if data.is_none() {
            // Nothing to do.
        } else if let Some(base_data) = &base_data {
            data.as_mut().expect("checked").jamo_ce32s = base_data.jamo_ce32s.clone();
        } else {
            return Err(icu("Missing Jamo CE32s for Hangul processing"));
        }

        index = Self::IX_ROOT_ELEMENTS_OFFSET;
        offset = in_indexes[index];
        length = in_indexes[index + 1] - offset;
        if length >= 4 {
            let root_elements_length = length / 4;
            let Some(d) = data.as_mut() else {
                return Err(icu("Root elements but no mappings"));
            };
            if root_elements_length <= CollationRootElements::IX_SEC_TER_BOUNDARIES {
                return Err(icu("Root elements array too short"));
            }
            let mut root_elements = vec![0i64; root_elements_length as usize];
            for e in root_elements.iter_mut() {
                *e = (in_bytes.get_int()? as i64) & 0xffffffff; // unsigned int -> long
            }
            let common_sec_ter =
                root_elements[CollationRootElements::IX_COMMON_SEC_AND_TER_CE as usize];
            if common_sec_ter != Collation::COMMON_SEC_AND_TER_CE as i64 {
                return Err(icu(
                    "Common sec/ter weights in base data differ from the hardcoded value",
                ));
            }
            let sec_ter_boundaries =
                root_elements[CollationRootElements::IX_SEC_TER_BOUNDARIES as usize];
            if ((sec_ter_boundaries as u64) >> 24) < CollationKeys::SEC_COMMON_HIGH as u64 {
                // [fixed last secondary common byte] is too low,
                // and secondary weights would collide with compressed common secondaries.
                return Err(icu("[fixed last secondary common byte] is too low"));
            }
            d.root_elements = Some(Arc::from(root_elements));
            length &= 3;
        }
        ICUBinary::skip_bytes(in_bytes, length)?;

        index = Self::IX_CONTEXTS_OFFSET;
        offset = in_indexes[index];
        length = in_indexes[index + 1] - offset;
        if length >= 2 {
            let Some(d) = data.as_mut() else {
                return Err(icu("Tailored contexts without tailored trie"));
            };
            d.contexts = Arc::from(ICUBinary::get_string(in_bytes, length / 2, length & 1)?);
        } else {
            ICUBinary::skip_bytes(in_bytes, length)?;
        }

        index = Self::IX_UNSAFE_BWD_OFFSET;
        offset = in_indexes[index];
        length = in_indexes[index + 1] - offset;
        if length >= 2 {
            let Some(d) = data.as_mut() else {
                return Err(icu("Unsafe-backward-set but no mappings"));
            };
            let mut unsafe_backward_set = match &base_data {
                None => {
                    // Create the unsafe-backward set for the root collator.
                    // Include all non-zero combining marks and trail surrogates.
                    // We do this at load time, rather than at build time,
                    // to simplify Unicode version bootstrapping:
                    // The root data builder only needs the new FractionalUCA.txt data,
                    // but it need not be built with a version of ICU already updated to
                    // the corresponding new Unicode Character Database.
                    //
                    // The following is an optimized version of
                    // new UnicodeSet("[[:^lccc=0:][\\udc00-\\udfff]]").
                    // It is faster and requires fewer code dependencies.
                    let mut set = UnicodeSet::new_range(0xdc00, 0xdfff)?; // trail surrogates
                    d.nfc_impl.add_lccc_chars(&mut set)?;
                    set
                }
                Some(base_data) => {
                    // Clone the root collator's set contents.
                    base_data
                        .unsafe_backward_set
                        .as_deref()
                        .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
                        .clone_as_thawed()
                }
            };
            // Add the ranges from the data file to the unsafe-backward set.
            let mut sset = USerializedSet::new();
            let unsafe_data = ICUBinary::get_chars(in_bytes, length / 2, length & 1)?;
            length = 0;
            sset.get_set(&unsafe_data, 0);
            let count = sset.count_ranges();
            let mut range = [0i32; 2];
            for i in 0..count {
                sset.get_range(i, &mut range)?;
                unsafe_backward_set.add_range(range[0], range[1])?;
            }
            // Mark each lead surrogate as "unsafe"
            // if any of its 1024 associated supplementary code points is "unsafe".
            let mut c = 0x10000;
            let mut lead = 0xd800;
            while lead < 0xdc00 {
                if !unsafe_backward_set.contains_none_range(c, c + 0x3ff)? {
                    unsafe_backward_set.add(lead)?;
                }
                lead += 1;
                c += 0x400;
            }
            unsafe_backward_set.freeze();
            let set = Arc::new(unsafe_backward_set);
            tailoring.unsafe_backward_set = Some(set.clone());
            d.unsafe_backward_set = Some(set);
        } else if data.is_none() {
            // Nothing to do.
        } else if let Some(base_data) = &base_data {
            // No tailoring-specific data: Alias the root collator's set.
            data.as_mut().expect("checked").unsafe_backward_set =
                base_data.unsafe_backward_set.clone();
        } else {
            return Err(icu("Missing unsafe-backward-set"));
        }
        ICUBinary::skip_bytes(in_bytes, length)?;

        // If the fast Latin format version is different,
        // or the version is set to 0 for "no fast Latin table",
        // then just always use the normal string comparison path.
        index = Self::IX_FAST_LATIN_TABLE_OFFSET;
        offset = in_indexes[index];
        length = in_indexes[index + 1] - offset;
        if let Some(d) = data.as_mut() {
            d.fast_latin_table = None;
            d.fast_latin_table_header = None;
            if ((in_indexes[Self::IX_OPTIONS] >> 16) & 0xff) == CollationFastLatin::VERSION {
                if length >= 2 {
                    let header0 = in_bytes.get_char()?;
                    let header_length = (header0 & 0xff) as i32;
                    let mut header = vec![0u16; header_length as usize];
                    header[0] = header0;
                    for h in header.iter_mut().skip(1) {
                        *h = in_bytes.get_char()?;
                    }
                    d.fast_latin_table_header = Some(Arc::from(header));
                    let table_length = length / 2 - header_length;
                    d.fast_latin_table = Some(Arc::from(ICUBinary::get_chars(
                        in_bytes,
                        table_length,
                        length & 1,
                    )?));
                    length = 0;
                    if (header0 >> 8) as i32 != CollationFastLatin::VERSION {
                        return Err(icu(
                            "Fast-Latin table version differs from version in data header",
                        ));
                    }
                } else if let Some(base_data) = &base_data {
                    d.fast_latin_table = base_data.fast_latin_table.clone();
                    d.fast_latin_table_header = base_data.fast_latin_table_header.clone();
                }
            }
        }
        ICUBinary::skip_bytes(in_bytes, length)?;

        index = Self::IX_SCRIPTS_OFFSET;
        offset = in_indexes[index];
        length = in_indexes[index + 1] - offset;
        if length >= 2 {
            let Some(d) = data.as_mut() else {
                return Err(icu("Script order data but no mappings"));
            };
            let scripts_length = length / 2;
            // CharBuffer inChars = inBytes.asCharBuffer(): a view from the
            // current position that does not move the byte buffer.
            let mut pos = in_bytes.position();
            let mut next_char = |in_bytes: &ByteBuffer| -> u16 {
                if pos + 2 > in_bytes.limit() {
                    panic!("java.nio.BufferUnderflowException");
                }
                let ch = in_bytes.get_char_at(pos);
                pos += 2;
                ch
            };
            d.num_scripts = next_char(in_bytes) as i32;
            // There must be enough entries for both arrays, including more than two range starts.
            let script_starts_length = scripts_length - (1 + d.num_scripts + 16);
            if script_starts_length <= 2 {
                return Err(icu("Script order data too short"));
            }
            let scripts_index: Vec<u16> = (0..d.num_scripts + 16)
                .map(|_| next_char(in_bytes))
                .collect();
            let script_starts: Vec<u16> = (0..script_starts_length)
                .map(|_| next_char(in_bytes))
                .collect();
            d.scripts_index = Arc::from(scripts_index);
            d.script_starts = Arc::from(script_starts);
            let ss = &d.script_starts;
            if !(ss[0] == 0
                && ss[1] as i32 == ((Collation::MERGE_SEPARATOR_BYTE + 1) << 8)
                && ss[(script_starts_length - 1) as usize] as i32
                    == (Collation::TRAIL_WEIGHT_BYTE << 8))
            {
                return Err(icu("Script order data not valid"));
            }
        } else if data.is_none() {
            // Nothing to do.
        } else if let Some(base_data) = &base_data {
            let d = data.as_mut().expect("checked");
            d.num_scripts = base_data.num_scripts;
            d.scripts_index = base_data.scripts_index.clone();
            d.script_starts = base_data.script_starts.clone();
        }
        ICUBinary::skip_bytes(in_bytes, length)?;

        index = Self::IX_COMPRESSIBLE_BYTES_OFFSET;
        offset = in_indexes[index];
        length = in_indexes[index + 1] - offset;
        if length >= 256 {
            let Some(d) = data.as_mut() else {
                return Err(icu(
                    "Data for compressible primary lead bytes but no mappings",
                ));
            };
            let mut compressible_bytes = vec![false; 256];
            for b in compressible_bytes.iter_mut() {
                *b = in_bytes.get()? != 0;
            }
            d.compressible_bytes = Arc::from(compressible_bytes);
            length -= 256;
        } else if data.is_none() {
            // Nothing to do.
        } else if let Some(base_data) = &base_data {
            data.as_mut().expect("checked").compressible_bytes =
                base_data.compressible_bytes.clone();
        } else {
            return Err(icu("Missing data for compressible primary lead bytes"));
        }
        ICUBinary::skip_bytes(in_bytes, length)?;

        index = Self::IX_RESERVED18_OFFSET;
        offset = in_indexes[index];
        length = in_indexes[index + 1] - offset;
        ICUBinary::skip_bytes(in_bytes, length)?;

        if let Some(d) = data {
            tailoring.set_owned_data(d);
        }

        let ts = tailoring.settings();
        let options = in_indexes[Self::IX_OPTIONS] & 0xffff;
        let mut fast_latin_primaries = vec![0u16; CollationFastLatin::LATIN_LIMIT as usize];
        let fast_latin_options =
            CollationFastLatin::get_options(tailoring.data(), ts, &mut fast_latin_primaries);
        if options == ts.options
            && ts.variable_top != 0
            && reorder_codes[..] == ts.reorder_codes[..]
            && fast_latin_options == ts.fast_latin_options
            && (fast_latin_options < 0 || fast_latin_primaries[..] == ts.fast_latin_primaries[..])
        {
            return Ok(());
        }

        let data = tailoring.data().clone();
        let settings = tailoring.settings.copy_on_write();
        settings.options = options;
        // Set variableTop from options and scripts data.
        settings.variable_top =
            data.get_last_primary_for_group(ReorderCodes::FIRST + settings.get_max_variable());
        if settings.variable_top == 0 {
            return Err(icu("The maxVariable could not be mapped to a variableTop"));
        }

        if reorder_codes_length != 0 {
            settings.alias_reordering(
                base_data.as_deref(),
                &reorder_codes,
                reorder_codes_length,
                reorder_table,
            )?;
        }

        let mut primaries = std::mem::take(&mut settings.fast_latin_primaries);
        settings.fast_latin_options =
            CollationFastLatin::get_options(&data, settings, &mut primaries);
        settings.fast_latin_primaries = primaries;
        Ok(())
    }
}
