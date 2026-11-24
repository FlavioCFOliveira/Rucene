// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.breakiter.DictionaryData`.

use rucene::error::Result;

use crate::jdk::to_rust_string;
use crate::r#impl::assert::Assert;
use crate::r#impl::breakiter::bytes_dictionary_matcher::BytesDictionaryMatcher;
use crate::r#impl::breakiter::chars_dictionary_matcher::CharsDictionaryMatcher;
use crate::r#impl::breakiter::dictionary_matcher::DictionaryMatcher;
use crate::r#impl::icu_binary::ICUBinary;
use crate::r#impl::icu_data::ICUData;
use crate::util::u_resource_bundle::UResourceBundle;

/// The loader of the break-iterator dictionaries (`brkitr/*.dict`).
///
/// Equivalent to the package-private final class
/// `com.ibm.icu.impl.breakiter.DictionaryData`.
pub struct DictionaryData;

impl DictionaryData {
    /// `TRIE_TYPE_BYTES`.
    pub const TRIE_TYPE_BYTES: i32 = 0;
    /// `TRIE_TYPE_UCHARS`.
    pub const TRIE_TYPE_UCHARS: i32 = 1;
    /// `TRIE_TYPE_MASK`.
    pub const TRIE_TYPE_MASK: i32 = 7;
    /// `TRIE_HAS_VALUES`.
    pub const TRIE_HAS_VALUES: i32 = 8;
    /// `TRANSFORM_NONE`.
    pub const TRANSFORM_NONE: i32 = 0;
    /// `TRANSFORM_TYPE_OFFSET`.
    pub const TRANSFORM_TYPE_OFFSET: i32 = 0x100_0000;
    /// `TRANSFORM_TYPE_MASK`.
    pub const TRANSFORM_TYPE_MASK: i32 = 0x7f00_0000;
    /// `TRANSFORM_OFFSET_MASK`.
    pub const TRANSFORM_OFFSET_MASK: i32 = 0x1f_ffff;

    /// `IX_STRING_TRIE_OFFSET`.
    pub const IX_STRING_TRIE_OFFSET: i32 = 0;
    /// `IX_RESERVED1_OFFSET`.
    pub const IX_RESERVED1_OFFSET: i32 = 1;
    /// `IX_RESERVED2_OFFSET`.
    pub const IX_RESERVED2_OFFSET: i32 = 2;
    /// `IX_TOTAL_SIZE`.
    pub const IX_TOTAL_SIZE: i32 = 3;
    /// `IX_TRIE_TYPE`.
    pub const IX_TRIE_TYPE: i32 = 4;
    /// `IX_TRANSFORM`.
    pub const IX_TRANSFORM: i32 = 5;
    /// `IX_RESERVED6`.
    pub const IX_RESERVED6: i32 = 6;
    /// `IX_RESERVED7`.
    pub const IX_RESERVED7: i32 = 7;
    /// `IX_COUNT`.
    pub const IX_COUNT: i32 = 8;

    /// `DATA_FORMAT_ID`: `"Dict"`.
    const DATA_FORMAT_ID: i32 = 0x4469_6374;

    /// `loadDictionaryFor(String dictType)`: loads the dictionary the
    /// `brkitr` root bundle names under `dictionaries/<dictType>`; `None`
    /// for an unknown trie type.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` for a missing bundle entry or data file,
    /// `IOException` for a malformed header, `IllegalStateException`
    /// (`Assert.assrt`) for inconsistent indexes.
    pub(crate) fn load_dictionary_for(
        dict_type: &str,
    ) -> Result<Option<Box<dyn DictionaryMatcher>>> {
        let rb = UResourceBundle::get_bundle_instance_default(Some(ICUData::ICU_BRKITR_BASE_NAME))?;
        let rb = rb.as_icu()?;
        let dict_file_name = rb.get_string_with_fallback(&format!("dictionaries/{dict_type}"))?;
        let dict_file_name = format!(
            "{}/{}",
            ICUData::ICU_BRKITR_NAME,
            to_rust_string(&dict_file_name)
        );
        let mut bytes = ICUBinary::get_required_data(&dict_file_name)?;
        ICUBinary::read_header(&mut bytes, Self::DATA_FORMAT_ID, None)?;
        let mut indexes = [0i32; Self::IX_COUNT as usize];
        // TODO: read indexes[IX_STRING_TRIE_OFFSET] first, then read a variable-length indexes[]
        for index in indexes.iter_mut() {
            *index = bytes.get_int()?;
        }
        let offset = indexes[Self::IX_STRING_TRIE_OFFSET as usize];
        Assert::assrt(offset >= (4 * Self::IX_COUNT))?;
        if offset > (4 * Self::IX_COUNT) {
            let diff = offset - (4 * Self::IX_COUNT);
            ICUBinary::skip_bytes(&mut bytes, diff)?;
        }
        let trie_type = indexes[Self::IX_TRIE_TYPE as usize] & Self::TRIE_TYPE_MASK;
        let total_size = indexes[Self::IX_TOTAL_SIZE as usize] - offset;
        let m: Option<Box<dyn DictionaryMatcher>> = if trie_type == Self::TRIE_TYPE_BYTES {
            let transform = indexes[Self::IX_TRANSFORM as usize];
            if total_size < 0 {
                // new byte[negative]
                return Err(rucene::error::LuceneError::unchecked(
                    "java.lang.NegativeArraySizeException",
                    total_size.to_string(),
                ));
            }
            let mut data = vec![0u8; total_size as usize];
            bytes.get_bytes(&mut data)?;
            Some(Box::new(BytesDictionaryMatcher::new(
                data.into_iter().map(|b| b as i8).collect(),
                transform,
            )?))
        } else if trie_type == Self::TRIE_TYPE_UCHARS {
            Assert::assrt(total_size % 2 == 0)?;
            let data = ICUBinary::get_string(&mut bytes, total_size / 2, total_size & 1)?;
            Some(Box::new(CharsDictionaryMatcher::new(data)))
        } else {
            None
        };
        Ok(m)
    }
}
