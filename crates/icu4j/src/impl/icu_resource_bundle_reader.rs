// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.ICUResourceBundleReader`.

use std::any::Any;
use std::sync::{Arc, LazyLock, Mutex};

use rucene::error::{LuceneError, LuceneErrorKind, Result};

use crate::jdk::{ByteBuffer, ClassLoader};
use crate::r#impl::cache_base::CacheBase;
use crate::r#impl::cache_value::CacheValue;
use crate::r#impl::icu_binary::ICUBinary;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::ICUResourceBundle;
use crate::r#impl::soft_cache::SoftCache;
use crate::r#impl::u_resource::{self, Key};
use crate::util::icu_exception::ICUException;
use crate::util::icu_unchecked_io_exception::ICUUncheckedIOException;
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::UResourceBundle;
use crate::util::u_resource_type_mismatch_exception::type_mismatch;
use crate::util::version_info::VersionInfo;

/// `DATA_FORMAT`: "ResB".
const DATA_FORMAT: i32 = 0x52657342;

/// `IsAcceptable.isDataVersionAcceptable(byte[] formatVersion)`.
fn is_acceptable(format_version: &[u8; 4]) -> bool {
    let fv0 = format_version[0] as i8;
    (fv0 == 1 && format_version[1] >= 1) || (2..=3).contains(&fv0)
}

/* indexes[] value names; indexes are generally 32-bit (Resource) indexes */
/// `URES_INDEX_LENGTH`: `[0]` contains `URES_INDEX_TOP`, the length of
/// `indexes[]`.
const URES_INDEX_LENGTH: i32 = 0;
/// `URES_INDEX_KEYS_TOP`: `[1]` contains the top of the key strings.
const URES_INDEX_KEYS_TOP: i32 = 1;
/// `URES_INDEX_BUNDLE_TOP`: `[3]` contains the top of the bundle.
const URES_INDEX_BUNDLE_TOP: i32 = 3;
/// `URES_INDEX_MAX_TABLE_LENGTH`.
const URES_INDEX_MAX_TABLE_LENGTH: i32 = 4;
/// `URES_INDEX_ATTRIBUTES`: `[5]` contains the attributes.
const URES_INDEX_ATTRIBUTES: i32 = 5;
/// `URES_INDEX_16BIT_TOP`: `[6]` contains the top of the 16-bit units.
const URES_INDEX_16BIT_TOP: i32 = 6;
/// `URES_INDEX_POOL_CHECKSUM`.
const URES_INDEX_POOL_CHECKSUM: i32 = 7;

/// `URES_ATT_NO_FALLBACK`.
const URES_ATT_NO_FALLBACK: i32 = 1;
/// `URES_ATT_IS_POOL_BUNDLE`: a pool bundle provides key strings that are
/// shared among several other bundles to reduce their total size (new in
/// formatVersion 2, ICU 4.4).
const URES_ATT_IS_POOL_BUNDLE: i32 = 2;
/// `URES_ATT_USES_POOL_BUNDLE`.
const URES_ATT_USES_POOL_BUNDLE: i32 = 4;

/// `LARGE_SIZE` (package-private): objects at least this large are cached
/// softly.
pub(crate) const LARGE_SIZE: i32 = 24;

/// This class reads the `*.res` resource bundle format.
///
/// Equivalent to `com.ibm.icu.impl.ICUResourceBundleReader`, the reading
/// of the binary ResB format (format versions 1.1 to 3) as ICU4J 77.1 does
/// it, including the pool bundle, the 16-bit units and the resource cache.
///
/// Java views the 16-bit units through a `CharBuffer` over the bundle's
/// bytes; the port copies them, in the bundle's byte order, into an array
/// at load time, which reads the same values. The resource cache holds its
/// large items through `SoftReference`s in Java; as everywhere in this
/// crate ([`CacheValue`]) they are never cleared, a behaviour the
/// `SoftReference` contract allows.
#[derive(Debug, Default)]
pub struct ICUResourceBundleReader {
    data_version: i32,
    // See the ResourceData struct in ICU4C/source/common/uresdata.h.
    bytes: Option<ByteBuffer>,
    key_bytes: Option<Arc<[i8]>>,
    b16_bit_units: Arc<[u16]>,
    pool_bundle_reader: Option<Arc<ICUResourceBundleReader>>,
    root_res: i32,
    local_key_limit: i32,
    pool_string_index_limit: i32,
    pool_string_index16_limit: i32,
    no_fallback: bool,
    is_pool_bundle: bool,
    uses_pool_bundle: bool,
    pool_check_sum: i32,
    resource_cache: Option<ResourceCache>,
}

/// `ReaderCacheKey` (private): the base name and locale ID, `null` read as
/// empty.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ReaderCacheKey {
    base_name: String,
    locale_id: String,
}

/// `CACHE`: the readers by base name and locale ID; `None` caches a
/// missing bundle (Java's `NULL_READER`).
///
/// Java's `CACHE` is an instance of the private class
/// `ICUResourceBundleReader.ReaderCache`, a `SoftCache` whose only member is
/// its `createInstance(ReaderCacheKey key, ClassLoader loader)` override; that
/// override is the closure given to [`SoftCache::new`] here.
#[allow(clippy::needless_late_init)] // declare-then-assign mirrors the Java source
static CACHE: LazyLock<
    SoftCache<ReaderCacheKey, Option<Arc<ICUResourceBundleReader>>, ClassLoader>,
> = LazyLock::new(|| {
    SoftCache::new(|key: &ReaderCacheKey, loader: ClassLoader| {
        let full_name =
            ICUResourceBundleReader::get_full_name(Some(&key.base_name), &key.locale_id);
        let result = (|| -> Result<Option<Arc<ICUResourceBundleReader>>> {
            let in_bytes;
            if key.base_name.starts_with(ICUData::ICU_BASE_NAME) {
                let item_path = &full_name[ICUData::ICU_BASE_NAME.len() + 1..];
                in_bytes =
                    match ICUBinary::get_data_with(Some(&loader), Some(&full_name), item_path)? {
                        Some(b) => b,
                        None => return Ok(None),
                    };
            } else {
                // Closed by getByteBufferFromInputStreamAndCloseStream().
                let mut stream = match ICUData::get_stream_loader(&loader, &full_name) {
                    Some(s) => s,
                    None => return Ok(None),
                };
                in_bytes =
                    ICUBinary::get_byte_buffer_from_input_stream_and_close_stream(&mut stream)?;
            }
            Ok(Some(Arc::new(ICUResourceBundleReader::new(
                in_bytes,
                &key.base_name,
                &key.locale_id,
                loader,
            )?)))
        })();
        match result {
            Ok(r) => Ok(Some(r)),
            Err(ex) if ex.is_io_exception() => {
                let msg = io_message(&ex);
                Err(ICUUncheckedIOException::with_message_and_cause(
                    format!("Data file {full_name} is corrupt - {msg}"),
                    ex,
                )
                .error())
            }
            Err(ex) => Err(ex),
        }
    })
});

/// `IOException.getMessage()`.
fn io_message(e: &LuceneError) -> String {
    match e.kind() {
        LuceneErrorKind::Io(io) => io.to_string(),
        _ => e.to_string(),
    }
}

/// `EMPTY_16_BIT_UNITS`: `"\0"`.
fn empty_16_bit_units() -> Arc<[u16]> {
    Arc::from(vec![0u16])
}

impl ICUResourceBundleReader {
    fn new(
        in_bytes: ByteBuffer,
        base_name: &str,
        _locale_id: &str,
        loader: ClassLoader,
    ) -> Result<Self> {
        let mut r = Self {
            b16_bit_units: empty_16_bit_units(),
            ..Self::default()
        };
        r.init(in_bytes)?;

        // set pool bundle if necessary
        if r.uses_pool_bundle {
            r.pool_bundle_reader = Self::get_reader(Some(base_name), Some("pool"), loader)?;
            match &r.pool_bundle_reader {
                Some(p) if p.is_pool_bundle => {
                    if p.pool_check_sum != r.pool_check_sum {
                        return Err(LuceneError::illegal_state(
                            "pool.res has a different checksum than this bundle",
                        ));
                    }
                }
                _ => return Err(LuceneError::illegal_state("pool.res is not a pool bundle")),
            }
        }
        Ok(r)
    }

    /// `getReader(String baseName, String localeID, ClassLoader root)`
    /// (package-private): `None` when the bundle does not exist.
    ///
    /// # Errors
    ///
    /// `ICUUncheckedIOException` for a corrupt data file; the
    /// `ICUException` or `IllegalStateException` of a malformed one.
    pub(crate) fn get_reader(
        base_name: Option<&str>,
        locale_id: Option<&str>,
        root: ClassLoader,
    ) -> Result<Option<Arc<ICUResourceBundleReader>>> {
        let info = ReaderCacheKey {
            base_name: base_name.unwrap_or("").to_string(),
            locale_id: locale_id.unwrap_or("").to_string(),
        };
        Ok(CACHE.get_instance(info, root)?.flatten())
    }

    fn bytes(&self) -> &ByteBuffer {
        self.bytes
            .as_ref()
            .expect("an initialized reader has bytes")
    }

    // See res_init() in ICU4C/source/common/uresdata.c.
    fn init(&mut self, mut in_bytes: ByteBuffer) -> Result<()> {
        self.data_version =
            ICUBinary::read_header(&mut in_bytes, DATA_FORMAT, Some(&is_acceptable))?;
        let major_format_version = in_bytes.get_at(16) as i32;
        let mut bytes = ICUBinary::slice_with_order(&in_bytes);
        let data_length = bytes.remaining();

        self.root_res = bytes.get_int_at(0);
        self.bytes = Some(bytes.clone());

        // Bundles with formatVersion 1.1 and later contain an indexes[] array.
        // We need it so that we can read the key string bytes up front, for lookup performance.

        // read the variable-length indexes[] array
        let indexes0 = self.get_indexes_int(URES_INDEX_LENGTH);
        let index_length = indexes0 & 0xff;
        if index_length <= URES_INDEX_MAX_TABLE_LENGTH {
            return Err(ICUException::with_message("not enough indexes").error());
        }
        let bundle_top;
        if data_length < ((1 + index_length) << 2) || {
            bundle_top = self.get_indexes_int(URES_INDEX_BUNDLE_TOP);
            data_length < (bundle_top << 2)
        } {
            return Err(ICUException::with_message("not enough bytes").error());
        }
        let mut max_offset = bundle_top - 1;

        if major_format_version >= 3 {
            // In formatVersion 1, the indexLength took up this whole int.
            // In version 2, bits 31..8 were reserved and always 0.
            // In version 3, they contain bits 23..0 of the poolStringIndexLimit.
            // Bits 27..24 are in indexes[URES_INDEX_ATTRIBUTES] bits 15..12.
            self.pool_string_index_limit = ((indexes0 as u32) >> 8) as i32;
        }
        if index_length > URES_INDEX_ATTRIBUTES {
            // determine if this resource bundle falls back to a parent bundle
            // along normal locale ID fallback
            let att = self.get_indexes_int(URES_INDEX_ATTRIBUTES);
            self.no_fallback = (att & URES_ATT_NO_FALLBACK) != 0;
            self.is_pool_bundle = (att & URES_ATT_IS_POOL_BUNDLE) != 0;
            self.uses_pool_bundle = (att & URES_ATT_USES_POOL_BUNDLE) != 0;
            self.pool_string_index_limit |= (att & 0xf000) << 12; // bits 15..12 -> 27..24
            self.pool_string_index16_limit = ((att as u32) >> 16) as i32;
        }

        let keys_bottom = 1 + index_length;
        let keys_top = self.get_indexes_int(URES_INDEX_KEYS_TOP);
        if keys_top > keys_bottom {
            // Deserialize the key strings up front.
            // Faster table item search at the cost of slower startup and some heap memory.
            let mut key_bytes;
            if self.is_pool_bundle {
                // Shift the key strings down:
                // Pool bundle key strings are used with a 0-based index,
                // unlike regular bundles' key strings for which indexes
                // are based on the start of the bundle data.
                key_bytes = vec![0u8; ((keys_top - keys_bottom) << 2) as usize];
                bytes.set_position(keys_bottom << 2)?;
            } else {
                self.local_key_limit = keys_top << 2;
                key_bytes = vec![0u8; self.local_key_limit as usize];
            }
            bytes.get_bytes(&mut key_bytes)?;
            let signed: Vec<i8> = key_bytes.into_iter().map(|b| b as i8).collect();
            self.key_bytes = Some(Arc::from(signed));
        }

        // Read the array of 16-bit units.
        if index_length > URES_INDEX_16BIT_TOP {
            let top_16bit = self.get_indexes_int(URES_INDEX_16BIT_TOP);
            if top_16bit > keys_top {
                let num_16bit_units = (top_16bit - keys_top) * 2;
                let start = keys_top << 2;
                // bytes.asCharBuffer() at position keysTop<<2, limit num16BitUnits
                let units: Vec<u16> = (0..num_16bit_units)
                    .map(|i| bytes.get_char_at(start + 2 * i))
                    .collect();
                self.b16_bit_units = Arc::from(units);
                max_offset |= num_16bit_units - 1;
            } else {
                self.b16_bit_units = empty_16_bit_units();
            }
        } else {
            self.b16_bit_units = empty_16_bit_units();
        }

        if index_length > URES_INDEX_POOL_CHECKSUM {
            self.pool_check_sum = self.get_indexes_int(URES_INDEX_POOL_CHECKSUM);
        }

        if !self.is_pool_bundle || self.b16_bit_units.len() > 1 {
            self.resource_cache = Some(ResourceCache::new(max_offset));
        }

        // Reset the position for future .asCharBuffer() etc.
        bytes.set_position(0)?;
        self.bytes = Some(bytes);
        Ok(())
    }

    fn get_indexes_int(&self, i: i32) -> i32 {
        self.bytes().get_int_at((1 + i) << 2)
    }

    /// `getVersion()` (package-private).
    pub fn get_version(&self) -> VersionInfo {
        ICUBinary::get_version_info_from_compact_int(self.data_version)
    }

    /// `getRootResource()` (package-private).
    pub(crate) fn get_root_resource(&self) -> i32 {
        self.root_res
    }

    /// `getNoFallback()` (package-private).
    pub(crate) fn get_no_fallback(&self) -> bool {
        self.no_fallback
    }

    /// `getUsesPoolBundle()` (package-private).
    pub fn get_uses_pool_bundle(&self) -> bool {
        self.uses_pool_bundle
    }

    /// `RES_GET_TYPE(int res)` (package-private).
    pub(crate) fn res_get_type(res: i32) -> i32 {
        ((res as u32) >> 28) as i32
    }

    fn res_get_offset(res: i32) -> i32 {
        res & 0x0fffffff
    }

    fn get_resource_byte_offset(offset: i32) -> i32 {
        offset << 2
    }

    /* get signed and unsigned integer values directly from the Resource handle */
    /// `RES_GET_INT(int res)` (package-private).
    pub(crate) fn res_get_int(res: i32) -> i32 {
        (res << 4) >> 4
    }

    /// `RES_GET_UINT(int res)` (package-private).
    pub(crate) fn res_get_uint(res: i32) -> i32 {
        res & 0x0fffffff
    }

    /// `URES_IS_ARRAY(int type)` (package-private).
    pub(crate) fn ures_is_array(ty: i32) -> bool {
        ty == UResourceBundle::ARRAY || ty == ICUResourceBundle::ARRAY16
    }

    /// `URES_IS_TABLE(int type)` (package-private).
    pub(crate) fn ures_is_table(ty: i32) -> bool {
        ty == UResourceBundle::TABLE
            || ty == ICUResourceBundle::TABLE16
            || ty == ICUResourceBundle::TABLE32
    }

    fn get_chars(&self, mut offset: i32, count: i32) -> Vec<u16> {
        let mut chars = Vec::with_capacity(count as usize);
        for _ in 0..count {
            chars.push(self.bytes().get_char_at(offset));
            offset += 2;
        }
        chars
    }

    fn get_int(&self, offset: i32) -> i32 {
        self.bytes().get_int_at(offset)
    }

    fn get_ints(&self, mut offset: i32, count: i32) -> Vec<i32> {
        let mut ints = Vec::with_capacity(count.max(0) as usize);
        for _ in 0..count {
            ints.push(self.bytes().get_int_at(offset));
            offset += 4;
        }
        ints
    }

    fn get_table16_key_offsets(&self, mut offset: i32) -> Vec<u16> {
        let length = self.b16_bit_units[offset as usize] as i32;
        offset += 1;
        if length > 0 {
            self.b16_bit_units[offset as usize..(offset + length) as usize].to_vec()
        } else {
            Vec::new()
        }
    }

    fn get_table_key_offsets(&self, offset: i32) -> Vec<u16> {
        let length = self.bytes().get_char_at(offset) as i32;
        if length > 0 {
            self.get_chars(offset + 2, length)
        } else {
            Vec::new()
        }
    }

    fn get_table32_key_offsets(&self, offset: i32) -> Vec<i32> {
        let length = self.get_int(offset);
        if length > 0 {
            self.get_ints(offset + 4, length)
        } else {
            Vec::new()
        }
    }

    fn make_key_string_from_bytes(key_bytes: &[i8], key_offset: i32) -> String {
        let mut end = key_offset as usize;
        while key_bytes[end] != 0 {
            end += 1;
        }
        // new String(keyBytes, keyOffset, len, StandardCharsets.ISO_8859_1)
        key_bytes[key_offset as usize..end]
            .iter()
            .map(|&b| b as u8 as char)
            .collect()
    }

    fn key_bytes(&self) -> &Arc<[i8]> {
        self.key_bytes
            .as_ref()
            .expect("a bundle with keys has key bytes")
    }

    fn pool(&self) -> &ICUResourceBundleReader {
        self.pool_bundle_reader
            .as_deref()
            .expect("java.lang.NullPointerException: the pool bundle reader is null")
    }

    fn get_key16_string(&self, key_offset: i32) -> String {
        if key_offset < self.local_key_limit {
            Self::make_key_string_from_bytes(self.key_bytes(), key_offset)
        } else {
            Self::make_key_string_from_bytes(
                self.pool().key_bytes(),
                key_offset - self.local_key_limit,
            )
        }
    }

    fn get_key32_string(&self, key_offset: i32) -> String {
        if key_offset >= 0 {
            Self::make_key_string_from_bytes(self.key_bytes(), key_offset)
        } else {
            Self::make_key_string_from_bytes(self.pool().key_bytes(), key_offset & 0x7fffffff)
        }
    }

    fn set_key_from_key16(&self, key_offset: i32, key: &mut Key) {
        if key_offset < self.local_key_limit {
            key.set_bytes(self.key_bytes(), key_offset);
        } else {
            key.set_bytes(self.pool().key_bytes(), key_offset - self.local_key_limit);
        }
    }

    fn set_key_from_key32(&self, key_offset: i32, key: &mut Key) {
        if key_offset >= 0 {
            key.set_bytes(self.key_bytes(), key_offset);
        } else {
            key.set_bytes(self.pool().key_bytes(), key_offset & 0x7fffffff);
        }
    }

    fn compare_keys(&self, key: &[u16], key_offset: u16) -> i32 {
        let key_offset = key_offset as i32;
        if key_offset < self.local_key_limit {
            ICUBinary::compare_keys_bytes(key, self.key_bytes(), key_offset)
        } else {
            ICUBinary::compare_keys_bytes(
                key,
                self.pool().key_bytes(),
                key_offset - self.local_key_limit,
            )
        }
    }

    fn compare_keys32(&self, key: &[u16], key_offset: i32) -> i32 {
        if key_offset >= 0 {
            ICUBinary::compare_keys_bytes(key, self.key_bytes(), key_offset)
        } else {
            ICUBinary::compare_keys_bytes(key, self.pool().key_bytes(), key_offset & 0x7fffffff)
        }
    }

    fn cache(&self) -> &ResourceCache {
        self.resource_cache
            .as_ref()
            .expect("java.lang.NullPointerException: the resource cache is null")
    }

    /// `getStringV2(int res)` (package-private).
    #[allow(clippy::needless_late_init)] // declare-then-assign mirrors the Java source
    pub(crate) fn get_string_v2(&self, res: i32) -> Vec<u16> {
        // Use the pool bundle's resource cache for pool bundle strings;
        // use the local bundle's cache for local strings.
        // The cache requires a resource word with the proper type,
        // and with an offset that is local to this bundle so that the offset fits
        // within the maximum number of bits for which the cache was constructed.
        debug_assert!(Self::res_get_type(res) == ICUResourceBundle::STRING_V2);
        let mut offset = Self::res_get_offset(res);
        debug_assert!(offset != 0); // handled by the caller
        if let Some(CacheItem::Str(s)) = self.cache().get(res) {
            return s.to_vec();
        }
        let units = &self.b16_bit_units;
        let s: Vec<u16>;
        let first = units[offset as usize] as i32;
        if (first & 0xfffffc00u32 as i32) != 0xdc00 {
            // C: if(!U16_IS_TRAIL(first)) {
            if first == 0 {
                return Vec::new(); // Should not occur, but is not forbidden.
            }
            let mut sb = vec![first as u16];
            loop {
                offset += 1;
                let c = units[offset as usize];
                if c == 0 {
                    break;
                }
                sb.push(c);
            }
            s = sb;
        } else {
            let length;
            if first < 0xdfef {
                length = first & 0x3ff;
                offset += 1;
            } else if first < 0xdfff {
                length = ((first - 0xdfef) << 16) | units[(offset + 1) as usize] as i32;
                offset += 2;
            } else {
                length = ((units[(offset + 1) as usize] as i32) << 16)
                    | units[(offset + 2) as usize] as i32;
                offset += 3;
            }
            s = units[offset as usize..(offset + length) as usize].to_vec();
        }
        let size = s.len() as i32 * 2;
        match self
            .cache()
            .put_if_absent(res, CacheItem::Str(Arc::from(s)), size)
        {
            CacheItem::Str(s) => s.to_vec(),
            _ => unreachable!("a string resource caches a string"),
        }
    }

    fn make_string_from_bytes(&self, mut offset: i32, length: i32) -> Vec<u16> {
        let mut sb = Vec::with_capacity(length.max(0) as usize);
        for _ in 0..length {
            sb.push(self.bytes().get_char_at(offset));
            offset += 2;
        }
        sb
    }

    /// `getString(int res)` (package-private): `None` when `res` is not a
    /// string.
    pub(crate) fn get_string(&self, res: i32) -> Option<Vec<u16>> {
        let mut offset = Self::res_get_offset(res);
        if res != offset /* RES_GET_TYPE(res) != URES_STRING */
            && Self::res_get_type(res) != ICUResourceBundle::STRING_V2
        {
            return None;
        }
        if offset == 0 {
            return Some(Vec::new());
        }
        if res != offset {
            // STRING_V2
            if offset < self.pool_string_index_limit {
                return Some(self.pool().get_string_v2(res));
            } else {
                return Some(self.get_string_v2(res - self.pool_string_index_limit));
            }
        }
        if let Some(CacheItem::Str(s)) = self.cache().get(res) {
            return Some(s.to_vec());
        }
        offset = Self::get_resource_byte_offset(offset);
        let length = self.get_int(offset);
        let s = self.make_string_from_bytes(offset + 4, length);
        let size = s.len() as i32 * 2;
        match self
            .cache()
            .put_if_absent(res, CacheItem::Str(Arc::from(s)), size)
        {
            CacheItem::Str(s) => Some(s.to_vec()),
            _ => unreachable!("a string resource caches a string"),
        }
    }

    /// `isNoInheritanceMarker(int res)` (private).
    fn is_no_inheritance_marker(&self, res: i32) -> bool {
        let mut offset = Self::res_get_offset(res);
        if offset == 0 {
            // empty string
        } else if res == offset {
            offset = Self::get_resource_byte_offset(offset);
            let b = self.bytes();
            return self.get_int(offset) == 3
                && b.get_char_at(offset + 4) == 0x2205
                && b.get_char_at(offset + 6) == 0x2205
                && b.get_char_at(offset + 8) == 0x2205;
        } else if Self::res_get_type(res) == ICUResourceBundle::STRING_V2 {
            if offset < self.pool_string_index_limit {
                return self.pool().is_string_v2_no_inheritance_marker(offset);
            } else {
                return self
                    .is_string_v2_no_inheritance_marker(offset - self.pool_string_index_limit);
            }
        }
        false
    }

    fn is_string_v2_no_inheritance_marker(&self, offset: i32) -> bool {
        let u = &self.b16_bit_units;
        let o = offset as usize;
        let first = u[o] as i32;
        if first == 0x2205 {
            // implicit length
            u[o + 1] == 0x2205 && u[o + 2] == 0x2205 && u[o + 3] == 0
        } else if first == 0xdc03 {
            // explicit length 3 (should not occur)
            u[o + 1] == 0x2205 && u[o + 2] == 0x2205 && u[o + 3] == 0x2205
        } else {
            // Assume that the string has not been stored with more length units than necessary.
            false
        }
    }

    /// `getAlias(int res)` (package-private): `None` when `res` is not an
    /// alias.
    pub(crate) fn get_alias(&self, res: i32) -> Option<Vec<u16>> {
        let mut offset = Self::res_get_offset(res);
        if Self::res_get_type(res) == ICUResourceBundle::ALIAS {
            if offset == 0 {
                Some(Vec::new())
            } else {
                if let Some(CacheItem::Str(s)) = self.cache().get(res) {
                    return Some(s.to_vec());
                }
                offset = Self::get_resource_byte_offset(offset);
                let length = self.get_int(offset);
                let s = self.make_string_from_bytes(offset + 4, length);
                match self
                    .cache()
                    .put_if_absent(res, CacheItem::Str(Arc::from(s)), length * 2)
                {
                    CacheItem::Str(s) => Some(s.to_vec()),
                    _ => unreachable!("an alias resource caches a string"),
                }
            }
        } else {
            None
        }
    }

    /// `getBinary(int res, byte[] ba)` (package-private): the bytes of a
    /// binary resource into `ba` when it has the right length, else into a
    /// new array; `None` when `res` is not binary.
    pub(crate) fn get_binary_bytes(&self, res: i32, ba: Option<Vec<u8>>) -> Option<Vec<u8>> {
        let mut offset = Self::res_get_offset(res);
        if Self::res_get_type(res) == UResourceBundle::BINARY {
            if offset == 0 {
                Some(Vec::new())
            } else {
                offset = Self::get_resource_byte_offset(offset);
                let length = self.get_int(offset);
                if length == 0 {
                    return Some(Vec::new());
                }
                // Not cached: The array would have to be cloned anyway because
                // the cache must not be writable via the returned reference.
                let mut ba = match ba {
                    Some(b) if b.len() == length as usize => b,
                    _ => vec![0u8; length as usize],
                };
                offset += 4;
                for (i, b) in ba.iter_mut().enumerate() {
                    *b = self.bytes().get_at(offset + i as i32) as u8;
                }
                Some(ba)
            }
        } else {
            None
        }
    }

    /// `getBinary(int res)` (package-private): a read-only buffer over the
    /// bytes of a binary resource; `None` when `res` is not binary.
    pub(crate) fn get_binary(&self, res: i32) -> Option<ByteBuffer> {
        let mut offset = Self::res_get_offset(res);
        if Self::res_get_type(res) == UResourceBundle::BINARY {
            if offset == 0 {
                // Don't just
                //   return emptyByteBuffer;
                // in case it matters whether the buffer's mark is defined or undefined.
                Some(ByteBuffer::wrap(Vec::new()))
            } else {
                // Not cached: The returned buffer is small (shares its bytes with the bundle)
                // and usually quickly discarded after use.
                // Also, even a cached buffer would have to be cloned because it is mutable
                // (position & mark).
                offset = Self::get_resource_byte_offset(offset);
                let length = self.get_int(offset);
                if length == 0 {
                    return Some(ByteBuffer::wrap(Vec::new()));
                }
                offset += 4;
                let mut result = self.bytes().duplicate();
                result.set_order(self.bytes().order());
                result
                    .set_position(offset)
                    .and_then(|r| r.set_limit(offset + length))
                    .expect("a binary resource lies inside its bundle");
                Some(ICUBinary::slice_with_order(&result))
            }
        } else {
            None
        }
    }

    /// `getIntVector(int res)` (package-private): `None` when `res` is not
    /// an int vector.
    pub(crate) fn get_int_vector(&self, res: i32) -> Option<Vec<i32>> {
        let mut offset = Self::res_get_offset(res);
        if Self::res_get_type(res) == UResourceBundle::INT_VECTOR {
            if offset == 0 {
                Some(Vec::new())
            } else {
                // Not cached: The array would have to be cloned anyway because
                // the cache must not be writable via the returned reference.
                offset = Self::get_resource_byte_offset(offset);
                let length = self.get_int(offset);
                Some(self.get_ints(offset + 4, length))
            }
        } else {
            None
        }
    }

    /// `getArray(int res)` (package-private): `None` when `res` is not an
    /// array.
    pub(crate) fn get_array(&self, res: i32) -> Option<Arc<Container>> {
        let ty = Self::res_get_type(res);
        if !Self::ures_is_array(ty) {
            return None;
        }
        let offset = Self::res_get_offset(res);
        if offset == 0 {
            return Some(EMPTY_ARRAY.clone());
        }
        if let Some(CacheItem::Container(c)) = self.cache().get(res) {
            return Some(c);
        }
        let array = if ty == UResourceBundle::ARRAY {
            Container::array32(self, offset)
        } else {
            Container::array16(self, offset)
        };
        match self
            .cache()
            .put_if_absent(res, CacheItem::Container(Arc::new(array)), 0)
        {
            CacheItem::Container(c) => Some(c),
            _ => unreachable!("an array resource caches a container"),
        }
    }

    /// `getTable(int res)` (package-private): `None` when `res` is not a
    /// table.
    #[allow(clippy::needless_late_init)] // declare-then-assign mirrors the Java source
    pub(crate) fn get_table(&self, res: i32) -> Option<Arc<Container>> {
        let ty = Self::res_get_type(res);
        if !Self::ures_is_table(ty) {
            return None;
        }
        let offset = Self::res_get_offset(res);
        if offset == 0 {
            return Some(EMPTY_TABLE.clone());
        }
        if let Some(CacheItem::Container(c)) = self.cache().get(res) {
            return Some(c);
        }
        let table;
        let size; // Use size = 0 to never use SoftReferences for Tables?
        if ty == UResourceBundle::TABLE {
            table = Container::table1632(self, offset);
            size = table.get_size() * 2;
        } else if ty == ICUResourceBundle::TABLE16 {
            table = Container::table16(self, offset);
            size = table.get_size() * 2;
        } else
        /* type == ICUResourceBundle.TABLE32 */
        {
            table = Container::table32(self, offset);
            size = table.get_size() * 4;
        }
        match self
            .cache()
            .put_if_absent(res, CacheItem::Container(Arc::new(table)), size)
        {
            CacheItem::Container(c) => Some(c),
            _ => unreachable!("a table resource caches a container"),
        }
    }

    /// `getFullName(String baseName, String localeName)`: the resource
    /// name of a bundle.
    pub fn get_full_name(base_name: Option<&str>, locale_name: &str) -> String {
        const ICU_RESOURCE_SUFFIX: &str = ".res";
        match base_name {
            None | Some("") => {
                if locale_name.is_empty() {
                    return ULocale::get_default().to_string();
                }
                format!("{locale_name}{ICU_RESOURCE_SUFFIX}")
            }
            Some(base_name) => {
                if !base_name.contains('.') {
                    if !base_name.ends_with('/') {
                        format!("{base_name}/{locale_name}{ICU_RESOURCE_SUFFIX}")
                    } else {
                        format!("{base_name}{locale_name}{ICU_RESOURCE_SUFFIX}")
                    }
                } else {
                    let base_name = base_name.replace('.', "/");
                    if locale_name.is_empty() {
                        format!("{base_name}{ICU_RESOURCE_SUFFIX}")
                    } else {
                        format!("{base_name}_{locale_name}{ICU_RESOURCE_SUFFIX}")
                    }
                }
            }
        }
    }
}

// ICUResource.Value --------------------------------------------------- ***

/// `PUBLIC_TYPES`: the public type of each internal resource type.
const PUBLIC_TYPES: [i32; 16] = [
    UResourceBundle::STRING,
    UResourceBundle::BINARY,
    UResourceBundle::TABLE,
    ICUResourceBundle::ALIAS,
    UResourceBundle::TABLE,  /* URES_TABLE32 */
    UResourceBundle::TABLE,  /* URES_TABLE16 */
    UResourceBundle::STRING, /* URES_STRING_V2 */
    UResourceBundle::INT,
    UResourceBundle::ARRAY,
    UResourceBundle::ARRAY, /* URES_ARRAY16 */
    UResourceBundle::NONE,
    UResourceBundle::NONE,
    UResourceBundle::NONE,
    UResourceBundle::NONE,
    UResourceBundle::INT_VECTOR,
    UResourceBundle::NONE,
];

/// The [`Value`](u_resource::Value) the reader enumerates with: a reader
/// and a resource word.
///
/// Equivalent to `ICUResourceBundleReader.ReaderValue` (package-private).
#[derive(Debug, Clone, Default)]
pub struct ReaderValue {
    /// `reader`.
    pub(crate) reader: Option<Arc<ICUResourceBundleReader>>,
    /// `res`.
    pub(crate) res: i32,
}

impl ReaderValue {
    /// `new ReaderValue()`.
    pub fn new() -> Self {
        Self::default()
    }

    fn reader(&self) -> &ICUResourceBundleReader {
        self.reader
            .as_deref()
            .expect("java.lang.NullPointerException: the value has no reader")
    }

    fn get_string_array_of(&self, array: &Container) -> Result<Vec<Vec<u16>>> {
        let mut result = Vec::with_capacity(array.size as usize);
        for i in 0..array.size {
            let r = array.get_container_resource(self.reader(), i);
            match self.reader().get_string(r) {
                Some(s) => result.push(s),
                None => return Err(type_mismatch("")),
            }
        }
        Ok(result)
    }
}

impl u_resource::Value for ReaderValue {
    fn get_type(&self) -> i32 {
        PUBLIC_TYPES[ICUResourceBundleReader::res_get_type(self.res) as usize]
    }

    fn get_string(&self) -> Result<Vec<u16>> {
        self.reader()
            .get_string(self.res)
            .ok_or_else(|| type_mismatch(""))
    }

    fn get_alias_string(&self) -> Result<Vec<u16>> {
        self.reader()
            .get_alias(self.res)
            .ok_or_else(|| type_mismatch(""))
    }

    fn get_int(&self) -> Result<i32> {
        if ICUResourceBundleReader::res_get_type(self.res) != UResourceBundle::INT {
            return Err(type_mismatch(""));
        }
        Ok(ICUResourceBundleReader::res_get_int(self.res))
    }

    fn get_u_int(&self) -> Result<i32> {
        if ICUResourceBundleReader::res_get_type(self.res) != UResourceBundle::INT {
            return Err(type_mismatch(""));
        }
        Ok(ICUResourceBundleReader::res_get_uint(self.res))
    }

    fn get_int_vector(&self) -> Result<Vec<i32>> {
        self.reader()
            .get_int_vector(self.res)
            .ok_or_else(|| type_mismatch(""))
    }

    fn get_binary(&self) -> Result<ByteBuffer> {
        self.reader()
            .get_binary(self.res)
            .ok_or_else(|| type_mismatch(""))
    }

    fn get_array(&self) -> Result<Arc<dyn u_resource::Array>> {
        match self.reader().get_array(self.res) {
            Some(a) => Ok(a),
            None => Err(type_mismatch("")),
        }
    }

    fn get_table(&self) -> Result<Arc<dyn u_resource::Table>> {
        match self.reader().get_table(self.res) {
            Some(t) => Ok(t),
            None => Err(type_mismatch("")),
        }
    }

    fn is_no_inheritance_marker(&self) -> bool {
        self.reader().is_no_inheritance_marker(self.res)
    }

    fn get_string_array(&self) -> Result<Vec<Vec<u16>>> {
        match self.reader().get_array(self.res) {
            Some(array) => self.get_string_array_of(&array),
            None => Err(type_mismatch("")),
        }
    }

    fn get_string_array_or_string_as_array(&self) -> Result<Vec<Vec<u16>>> {
        if let Some(array) = self.reader().get_array(self.res) {
            return self.get_string_array_of(&array);
        }
        if let Some(s) = self.reader().get_string(self.res) {
            return Ok(vec![s]);
        }
        Err(type_mismatch(""))
    }

    fn get_string_or_first_of_array(&self) -> Result<Vec<u16>> {
        if let Some(s) = self.reader().get_string(self.res) {
            return Ok(s);
        }
        if let Some(array) = self.reader().get_array(self.res) {
            if array.size > 0 {
                let r = array.get_container_resource(self.reader(), 0);
                if let Some(s) = self.reader().get_string(r) {
                    return Ok(s);
                }
            }
        }
        Err(type_mismatch(""))
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// `(ReaderValue) value`.
fn reader_value(value: &mut dyn u_resource::Value) -> &mut ReaderValue {
    value
        .as_any_mut()
        .downcast_mut::<ReaderValue>()
        .expect("java.lang.ClassCastException: the value is not a ReaderValue")
}

// Container value classes --------------------------------------------- ***

/// The kind of a [`Container`]: Java's subclass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerKind {
    /// `new Array()` (`EMPTY_ARRAY`).
    EmptyArray,
    /// `new Table()` (`EMPTY_TABLE`).
    EmptyTable,
    /// `Array32`.
    Array32,
    /// `Array16`.
    Array16,
    /// `Table1632`.
    Table1632,
    /// `Table16`.
    Table16,
    /// `Table32`.
    Table32,
}

/// An array or table resource of a bundle.
///
/// Equivalent to `ICUResourceBundleReader.Container` (package-private) and
/// its subclasses `Array`, `Array32`, `Array16`, `Table`, `Table1632`,
/// `Table16` and `Table32`, distinguished by [`ContainerKind`].
///
/// Equivalent to `com.ibm.icu.impl.ICUResourceBundleReader.Array`
/// (package-private, `ICUResourceBundleReader.java:977-988`).
///
/// Equivalent to `com.ibm.icu.impl.ICUResourceBundleReader.Table`
/// (package-private, `ICUResourceBundleReader.java:1010-1079`).
#[derive(Debug, Clone)]
pub struct Container {
    kind: ContainerKind,
    size: i32,
    items_offset: i32,
    key_offsets: Option<Vec<u16>>,
    key32_offsets: Option<Vec<i32>>,
}

/// `EMPTY_ARRAY`.
static EMPTY_ARRAY: LazyLock<Arc<Container>> = LazyLock::new(|| {
    Arc::new(Container {
        kind: ContainerKind::EmptyArray,
        size: 0,
        items_offset: 0,
        key_offsets: None,
        key32_offsets: None,
    })
});

/// `EMPTY_TABLE`.
static EMPTY_TABLE: LazyLock<Arc<Container>> = LazyLock::new(|| {
    Arc::new(Container {
        kind: ContainerKind::EmptyTable,
        size: 0,
        items_offset: 0,
        key_offsets: None,
        key32_offsets: None,
    })
});

/// `URESDATA_ITEM_NOT_FOUND`.
const URESDATA_ITEM_NOT_FOUND: i32 = -1;

impl Container {
    fn array32(reader: &ICUResourceBundleReader, offset: i32) -> Self {
        let offset = ICUResourceBundleReader::get_resource_byte_offset(offset);
        Self {
            kind: ContainerKind::Array32,
            size: reader.get_int(offset),
            items_offset: offset + 4,
            key_offsets: None,
            key32_offsets: None,
        }
    }

    fn array16(reader: &ICUResourceBundleReader, offset: i32) -> Self {
        Self {
            kind: ContainerKind::Array16,
            size: reader.b16_bit_units[offset as usize] as i32,
            items_offset: offset + 1,
            key_offsets: None,
            key32_offsets: None,
        }
    }

    fn table1632(reader: &ICUResourceBundleReader, offset: i32) -> Self {
        let offset = ICUResourceBundleReader::get_resource_byte_offset(offset);
        let key_offsets = reader.get_table_key_offsets(offset);
        let size = key_offsets.len() as i32;
        Self {
            kind: ContainerKind::Table1632,
            size,
            items_offset: offset + 2 * ((size + 2) & !1), // Skip padding for 4-alignment.
            key_offsets: Some(key_offsets),
            key32_offsets: None,
        }
    }

    fn table16(reader: &ICUResourceBundleReader, offset: i32) -> Self {
        let key_offsets = reader.get_table16_key_offsets(offset);
        let size = key_offsets.len() as i32;
        Self {
            kind: ContainerKind::Table16,
            size,
            items_offset: offset + 1 + size,
            key_offsets: Some(key_offsets),
            key32_offsets: None,
        }
    }

    fn table32(reader: &ICUResourceBundleReader, offset: i32) -> Self {
        let offset = ICUResourceBundleReader::get_resource_byte_offset(offset);
        let key32_offsets = reader.get_table32_key_offsets(offset);
        let size = key32_offsets.len() as i32;
        Self {
            kind: ContainerKind::Table32,
            size,
            items_offset: offset + 4 * (1 + size),
            key_offsets: None,
            key32_offsets: Some(key32_offsets),
        }
    }

    /// Whether this is a table (Java's `instanceof Table`).
    pub fn is_table(&self) -> bool {
        matches!(
            self.kind,
            ContainerKind::EmptyTable
                | ContainerKind::Table1632
                | ContainerKind::Table16
                | ContainerKind::Table32
        )
    }

    /// `getSize()`.
    pub fn get_size(&self) -> i32 {
        self.size
    }

    /// `getContainerResource(ICUResourceBundleReader reader, int index)`
    /// (package-private): `RES_BOGUS` when `index` is out of range.
    pub(crate) fn get_container_resource(
        &self,
        reader: &ICUResourceBundleReader,
        index: i32,
    ) -> i32 {
        match self.kind {
            ContainerKind::EmptyArray | ContainerKind::EmptyTable => ICUResourceBundle::RES_BOGUS,
            ContainerKind::Array32 | ContainerKind::Table1632 | ContainerKind::Table32 => {
                self.get_container32_resource(reader, index)
            }
            ContainerKind::Array16 | ContainerKind::Table16 => {
                self.get_container16_resource(reader, index)
            }
        }
    }

    fn get_container16_resource(&self, reader: &ICUResourceBundleReader, index: i32) -> i32 {
        if index < 0 || self.size <= index {
            return ICUResourceBundle::RES_BOGUS;
        }
        let mut res16 = reader.b16_bit_units[(self.items_offset + index) as usize] as i32;
        if res16 < reader.pool_string_index16_limit {
            // Pool string, nothing to do.
        } else {
            // Local string, adjust the 16-bit offset to a regular one,
            // with a larger pool string index limit.
            res16 = res16 - reader.pool_string_index16_limit + reader.pool_string_index_limit;
        }
        (ICUResourceBundle::STRING_V2 << 28) | res16
    }

    fn get_container32_resource(&self, reader: &ICUResourceBundleReader, index: i32) -> i32 {
        if index < 0 || self.size <= index {
            return ICUResourceBundle::RES_BOGUS;
        }
        reader.get_int(self.items_offset + 4 * index)
    }

    /// `getResource(ICUResourceBundleReader reader, String resKey)`
    /// (package-private): for an array, the key is an index
    /// (`Integer.parseInt`); for a table, it is looked up.
    ///
    /// # Errors
    ///
    /// `NumberFormatException` for an array key that is not a number.
    pub(crate) fn get_resource(
        &self,
        reader: &ICUResourceBundleReader,
        res_key: &str,
    ) -> Result<i32> {
        if self.is_table() {
            let key: Vec<u16> = res_key.encode_utf16().collect();
            Ok(self.get_container_resource(reader, self.find_table_item(reader, &key)))
        } else {
            let units: Vec<u16> = res_key.encode_utf16().collect();
            let index = crate::jdk::parse_int(&units, 10)?;
            Ok(self.get_container_resource(reader, index))
        }
    }

    /// `Table.getKey(ICUResourceBundleReader reader, int index)`
    /// (package-private): `None` when `index` is out of range.
    pub(crate) fn get_key(&self, reader: &ICUResourceBundleReader, index: i32) -> Option<String> {
        if index < 0 || self.size <= index {
            return None;
        }
        Some(match &self.key_offsets {
            Some(k) => reader.get_key16_string(k[index as usize] as i32),
            None => reader.get_key32_string(
                self.key32_offsets
                    .as_ref()
                    .expect("a table has key offsets")[index as usize],
            ),
        })
    }

    /// `Table.findTableItem(ICUResourceBundleReader reader, CharSequence
    /// key)` (package-private): binary search, `-1` when not found.
    pub(crate) fn find_table_item(&self, reader: &ICUResourceBundleReader, key: &[u16]) -> i32 {
        /* do a binary search for the key */
        let mut start = 0i32;
        let mut limit = self.size;
        while start < limit {
            let mid = ((start + limit) as u32 >> 1) as i32;
            let result = match &self.key_offsets {
                Some(k) => reader.compare_keys(key, k[mid as usize]),
                None => reader.compare_keys32(
                    key,
                    self.key32_offsets
                        .as_ref()
                        .expect("a table has key offsets")[mid as usize],
                ),
            };
            if result < 0 {
                limit = mid;
            } else if result > 0 {
                start = mid + 1;
            } else {
                /* We found it! */
                return mid;
            }
        }
        URESDATA_ITEM_NOT_FOUND /* not found or table is empty. */
    }
}

impl u_resource::Array for Container {
    fn get_size(&self) -> i32 {
        self.size
    }

    fn get_value(&self, i: i32, value: &mut dyn u_resource::Value) -> bool {
        if 0 <= i && i < self.size {
            let reader_value = reader_value(value);
            let reader = reader_value
                .reader
                .clone()
                .expect("a reader value has a reader");
            reader_value.res = self.get_container_resource(&reader, i);
            return true;
        }
        false
    }
}

impl u_resource::Table for Container {
    fn get_size(&self) -> i32 {
        self.size
    }

    fn get_key_and_value(&self, i: i32, key: &mut Key, value: &mut dyn u_resource::Value) -> bool {
        if 0 <= i && i < self.size {
            let reader_value = reader_value(value);
            let reader = reader_value
                .reader
                .clone()
                .expect("a reader value has a reader");
            match &self.key_offsets {
                Some(k) => reader.set_key_from_key16(k[i as usize] as i32, key),
                None => reader.set_key_from_key32(
                    self.key32_offsets
                        .as_ref()
                        .expect("a table has key offsets")[i as usize],
                    key,
                ),
            }
            reader_value.res = self.get_container_resource(&reader, i);
            return true;
        }
        false
    }

    fn find_value(&self, key: &[u16], value: &mut dyn u_resource::Value) -> bool {
        let reader_value = reader_value(value);
        let reader = reader_value
            .reader
            .clone()
            .expect("a reader value has a reader");
        let i = self.find_table_item(&reader, key);
        if i >= 0 {
            reader_value.res = self.get_container_resource(&reader, i);
            true
        } else {
            false
        }
    }
}

// Resource cache ------------------------------------------------------ ***

/// An item of the resource cache: a string (also an alias string), an
/// array or table container, or a child level of the trie.
#[derive(Debug, Clone)]
enum CacheItem {
    Str(Arc<[u16]>),
    Container(Arc<Container>),
}

/// A slot of a cache level: an item or a child level.
#[derive(Debug, Clone)]
enum Slot {
    Empty,
    Item(CacheItem),
    Level(Box<Level>),
}

/// Number of items to be stored in a simple array with binary search and
/// insertion sort.
const SIMPLE_LENGTH: usize = 32;

// When more than SIMPLE_LENGTH items are cached,
// then switch to a trie-like tree of levels with different array lengths.
const ROOT_BITS: i32 = 7;
const NEXT_BITS: i32 = 6;

/// `storeDirectly(int size)`.
fn store_directly(size: i32) -> bool {
    size < LARGE_SIZE || CacheValue::<()>::future_instances_will_be_strong()
}

/// `ResourceCache.Level`.
#[derive(Debug, Clone)]
struct Level {
    level_bits_list: i32,
    shift: i32,
    mask: i32,
    keys: Vec<i32>,
    values: Vec<Slot>,
}

impl Level {
    fn new(level_bits_list: i32, shift: i32) -> Self {
        let bits = level_bits_list & 0xf;
        debug_assert!(bits != 0);
        let length = 1usize << bits;
        Self {
            level_bits_list,
            shift,
            mask: length as i32 - 1,
            keys: vec![0; length],
            values: vec![Slot::Empty; length],
        }
    }

    fn get(&self, key: i32) -> Option<CacheItem> {
        let index = ((key >> self.shift) & self.mask) as usize;
        let k = self.keys[index];
        if k == key {
            return match &self.values[index] {
                Slot::Item(i) => Some(i.clone()),
                _ => None,
            };
        }
        if k == 0 {
            if let Slot::Level(level) = &self.values[index] {
                return level.get(key);
            }
        }
        None
    }

    fn put_if_absent(&mut self, key: i32, item: CacheItem, size: i32) -> CacheItem {
        let index = ((key >> self.shift) & self.mask) as usize;
        let k = self.keys[index];
        if k == key {
            return put_if_cleared(&mut self.values, index, item, size);
        }
        if k == 0 {
            if let Slot::Level(level) = &mut self.values[index] {
                return level.put_if_absent(key, item, size);
            }
            self.keys[index] = key;
            // storeDirectly(size) ? item : new SoftReference<>(item)
            let _ = store_directly(size);
            self.values[index] = Slot::Item(item.clone());
            return item;
        }
        // Collision: Add a child level, move the old item there,
        // and then insert the current item.
        let mut level = Level::new(
            self.level_bits_list >> 4,
            self.shift + (self.level_bits_list & 0xf),
        );
        let i = ((k >> level.shift) & level.mask) as usize;
        level.keys[i] = k;
        level.values[i] = std::mem::replace(&mut self.values[index], Slot::Empty);
        self.keys[index] = 0;
        let result = level.put_if_absent(key, item, size);
        self.values[index] = Slot::Level(Box::new(level));
        result
    }
}

/// `putIfCleared(Object[] values, int index, Object item, int size)`: the
/// value at `index`, which a cleared soft reference would have replaced
/// with `item` (never, see [`ICUResourceBundleReader`]).
fn put_if_cleared(values: &mut [Slot], index: usize, item: CacheItem, _size: i32) -> CacheItem {
    match &values[index] {
        Slot::Item(v) => v.clone(),
        _ => {
            values[index] = Slot::Item(item.clone());
            item
        }
    }
}

/// The mutable state of the resource cache.
#[derive(Debug)]
struct CacheState {
    // Simple table, used when length >= 0.
    keys: Vec<i32>,
    values: Vec<Slot>,
    length: i32,
    // Trie-like tree of levels, used when length < 0.
    root_level: Option<Level>,
}

/// Cache of the objects made from resources: strings, arrays and tables.
///
/// Equivalent to `ICUResourceBundleReader.ResourceCache` (private); its
/// `synchronized` methods hold a mutex.
#[derive(Debug)]
struct ResourceCache {
    max_offset_bits: i32,
    level_bits_list: i32,
    state: Mutex<CacheState>,
}

impl ResourceCache {
    #[allow(clippy::needless_late_init)] // declare-then-assign mirrors the Java source
    fn new(mut max_offset: i32) -> Self {
        debug_assert!(max_offset != 0);
        let mut max_offset_bits = 28;
        while max_offset <= 0x7ffffff {
            max_offset <<= 1;
            max_offset_bits -= 1;
        }
        let mut key_bits = max_offset_bits + 2; // +2 for mini type: at most 30 bits used in a key
                                                // Precompute for each level the number of bits it handles.
        let level_bits_list;
        if key_bits <= ROOT_BITS {
            level_bits_list = key_bits;
        } else if key_bits < (ROOT_BITS + 3) {
            level_bits_list = 0x30 | (key_bits - 3);
        } else {
            let mut list = ROOT_BITS;
            key_bits -= ROOT_BITS;
            let mut shift = 4;
            loop {
                if key_bits <= NEXT_BITS {
                    list |= key_bits << shift;
                    break;
                } else if key_bits < (NEXT_BITS + 3) {
                    list |= (0x30 | (key_bits - 3)) << shift;
                    break;
                } else {
                    list |= NEXT_BITS << shift;
                    key_bits -= NEXT_BITS;
                    shift += 4;
                }
            }
            level_bits_list = list;
        }
        Self {
            max_offset_bits,
            level_bits_list,
            state: Mutex::new(CacheState {
                keys: vec![0; SIMPLE_LENGTH],
                values: vec![Slot::Empty; SIMPLE_LENGTH],
                length: 0,
                root_level: None,
            }),
        }
    }

    fn make_key(&self, res: i32) -> i32 {
        // It is possible for resources of different types in the 16-bit array
        // to share a start offset; distinguish between those with a 2-bit value,
        // as a tie-breaker in the bits just above the highest possible offset.
        // It is not possible for "regular" resources of different types
        // to share a start offset with each other,
        // but offsets for 16-bit and "regular" resources overlap;
        // use 2-bit value 0 for "regular" resources.
        let ty = ICUResourceBundleReader::res_get_type(res);
        let mini_type = if ty == ICUResourceBundle::STRING_V2 {
            1
        } else if ty == ICUResourceBundle::TABLE16 {
            3
        } else if ty == ICUResourceBundle::ARRAY16 {
            2
        } else {
            0
        };
        ICUResourceBundleReader::res_get_offset(res) | (mini_type << self.max_offset_bits)
    }

    /// `Arrays.binarySearch(keys, 0, length, key)`.
    fn find_simple(state: &CacheState, key: i32) -> i32 {
        let mut low = 0i32;
        let mut high = state.length - 1;
        while low <= high {
            let mid = ((low + high) as u32 >> 1) as i32;
            let mid_val = state.keys[mid as usize];
            if mid_val < key {
                low = mid + 1;
            } else if mid_val > key {
                high = mid - 1;
            } else {
                return mid; // key found
            }
        }
        -(low + 1) // key not found.
    }

    fn get(&self, res: i32) -> Option<CacheItem> {
        // Integers and empty resources need not be cached.
        // The cache itself uses res=0 for "no match".
        debug_assert!(ICUResourceBundleReader::res_get_offset(res) != 0);
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.length >= 0 {
            let index = Self::find_simple(&state, res);
            if index >= 0 {
                match &state.values[index as usize] {
                    Slot::Item(i) => Some(i.clone()),
                    _ => None,
                }
            } else {
                None
            }
        } else {
            state
                .root_level
                .as_ref()
                .expect("a grown cache has a root level")
                .get(self.make_key(res))
        }
    }

    fn put_if_absent(&self, res: i32, item: CacheItem, size: i32) -> CacheItem {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.length >= 0 {
            let index = Self::find_simple(&state, res);
            if index >= 0 {
                return put_if_cleared(&mut state.values, index as usize, item, size);
            } else if (state.length as usize) < SIMPLE_LENGTH {
                let index = !index as usize;
                let length = state.length as usize;
                if index < length {
                    state.keys.copy_within(index..length, index + 1);
                    let slot = state.values.remove(length);
                    state.values.insert(index, slot);
                }
                state.length += 1;
                state.keys[index] = res;
                let _ = store_directly(size);
                state.values[index] = Slot::Item(item.clone());
                return item;
            } else
            /* not found && length == SIMPLE_LENGTH */
            {
                // Grow to become trie-like.
                let mut root_level = Level::new(self.level_bits_list, 0);
                for i in 0..SIMPLE_LENGTH {
                    let key = self.make_key(state.keys[i]);
                    if let Slot::Item(v) = std::mem::replace(&mut state.values[i], Slot::Empty) {
                        root_level.put_if_absent(key, v, 0);
                    }
                }
                state.keys = Vec::new();
                state.values = Vec::new();
                state.length = -1;
                state.root_level = Some(root_level);
            }
        }
        let key = self.make_key(res);
        state
            .root_level
            .as_mut()
            .expect("a grown cache has a root level")
            .put_if_absent(key, item, size)
    }
}
