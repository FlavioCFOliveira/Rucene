// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.UResource`: the low-level enumeration interfaces of
//! the resource bundle reader.

use std::any::Any;
use std::cmp::Ordering;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::jdk::ByteBuffer;
use crate::util::u_resource_bundle::UResourceBundle;

/// Represents a resource bundle item's key string: a view of the
/// NUL-terminated invariant-character key bytes of a bundle, or a string
/// set by the caller.
///
/// Equivalent to `com.ibm.icu.impl.UResource.Key`. Java's `CharSequence`
/// methods index the key's bytes, each byte widened to a `char` as Java's
/// `(char)byte` cast does (sign extension).
#[derive(Debug, Clone, Default)]
pub struct Key {
    bytes: Option<Arc<[i8]>>,
    offset: i32,
    length: i32,
    s: Option<String>,
}

impl Key {
    /// `new Key()`: the empty key.
    pub fn new() -> Self {
        Self {
            bytes: None,
            offset: 0,
            length: 0,
            s: Some(String::new()),
        }
    }

    /// `new Key(String s)`.
    ///
    /// # Errors
    ///
    /// As [`set_string`](Self::set_string).
    pub fn of(s: &str) -> Result<Self> {
        let mut k = Self::new();
        k.set_string(s)?;
        Ok(k)
    }

    fn of_bytes(key_bytes: Arc<[i8]>, key_offset: i32, key_length: i32) -> Self {
        Self {
            bytes: Some(key_bytes),
            offset: key_offset,
            length: key_length,
            s: None,
        }
    }

    /// `setBytes(byte[] keyBytes, int keyOffset)`: the key is the
    /// NUL-terminated bytes at `keyOffset`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` when the key has no terminator.
    pub fn set_bytes(&mut self, key_bytes: &Arc<[i8]>, key_offset: i32) -> &mut Self {
        self.bytes = Some(key_bytes.clone());
        self.offset = key_offset;
        self.length = 0;
        while key_bytes[(key_offset + self.length) as usize] != 0 {
            self.length += 1;
        }
        self.s = None;
        self
    }

    /// `setToEmpty()`.
    pub fn set_to_empty(&mut self) -> &mut Self {
        self.bytes = None;
        self.offset = 0;
        self.length = 0;
        self.s = Some(String::new());
        self
    }

    /// `setString(String s)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("\"<s>\" is not an ASCII string")`.
    pub fn set_string(&mut self, s: &str) -> Result<&mut Self> {
        if s.is_empty() {
            self.set_to_empty();
        } else {
            let mut bytes = Vec::with_capacity(s.len());
            for c in s.encode_utf16() {
                if c <= 0x7f {
                    bytes.push(c as i8);
                } else {
                    return Err(LuceneError::illegal_argument(format!(
                        "\"{s}\" is not an ASCII string"
                    )));
                }
            }
            self.length = bytes.len() as i32;
            self.bytes = Some(Arc::from(bytes));
            self.offset = 0;
            self.s = Some(s.to_string());
        }
        Ok(self)
    }

    /// `charAt(int i)`.
    pub fn char_at(&self, i: i32) -> u16 {
        debug_assert!(0 <= i && i < self.length);
        self.byte(i) as i16 as u16
    }

    fn byte(&self, i: i32) -> i8 {
        self.bytes.as_ref().expect("a non-empty key has bytes")[(self.offset + i) as usize]
    }

    /// `length()`.
    pub fn length(&self) -> i32 {
        self.length
    }

    /// `subSequence(int start, int end)`.
    pub fn sub_sequence(&self, start: i32, end: i32) -> Key {
        debug_assert!(0 <= start && start < self.length);
        debug_assert!(start <= end && end <= self.length);
        Key::of_bytes(
            self.bytes.clone().expect("a non-empty key has bytes"),
            self.offset + start,
            end - start,
        )
    }

    /// `toString()`.
    pub fn to_java_string(&mut self) -> String {
        if self.s.is_none() {
            self.s = Some(self.internal_sub_string(0, self.length));
        }
        self.s.clone().unwrap_or_default()
    }

    /// `toString()` without caching the string.
    pub fn as_string(&self) -> String {
        match &self.s {
            Some(s) => s.clone(),
            None => self.internal_sub_string(0, self.length),
        }
    }

    fn internal_sub_string(&self, start: i32, end: i32) -> String {
        let units: Vec<u16> = (start..end).map(|i| self.byte(i) as i16 as u16).collect();
        String::from_utf16_lossy(&units)
    }

    /// `substring(int start)`.
    pub fn substring(&self, start: i32) -> String {
        debug_assert!(0 <= start && start < self.length);
        self.internal_sub_string(start, self.length)
    }

    /// `substring(int start, int end)`.
    pub fn substring_range(&self, start: i32, end: i32) -> String {
        debug_assert!(0 <= start && start < self.length);
        debug_assert!(start <= end && end <= self.length);
        self.internal_sub_string(start, end)
    }

    fn region_matches_bytes(&self, other: &Key, n: i32) -> bool {
        for i in 0..n {
            if self.byte(i) != other.byte(i) {
                return false;
            }
        }
        true
    }

    fn region_matches_cs(&self, start: i32, cs: &[u16], n: i32) -> bool {
        for i in 0..n {
            if self.byte(start + i) as i32 != cs[i as usize] as i32 {
                return false;
            }
        }
        true
    }

    /// `contentEquals(CharSequence cs)`.
    pub fn content_equals(&self, cs: &str) -> bool {
        let cs: Vec<u16> = cs.encode_utf16().collect();
        cs.len() as i32 == self.length && self.region_matches_cs(0, &cs, self.length)
    }

    /// `startsWith(CharSequence cs)`.
    pub fn starts_with(&self, cs: &str) -> bool {
        let cs: Vec<u16> = cs.encode_utf16().collect();
        let cs_length = cs.len() as i32;
        cs_length <= self.length && self.region_matches_cs(0, &cs, cs_length)
    }

    /// `endsWith(CharSequence cs)`.
    pub fn ends_with(&self, cs: &str) -> bool {
        let cs: Vec<u16> = cs.encode_utf16().collect();
        let cs_length = cs.len() as i32;
        cs_length <= self.length && self.region_matches_cs(self.length - cs_length, &cs, cs_length)
    }

    /// `regionMatches(int start, CharSequence cs)`.
    pub fn region_matches(&self, start: i32, cs: &str) -> bool {
        let cs: Vec<u16> = cs.encode_utf16().collect();
        let cs_length = cs.len() as i32;
        cs_length == (self.length - start) && self.region_matches_cs(start, &cs, cs_length)
    }

    /// `hashCode()`: ICU's formula, which multiplies in the first byte at
    /// every step.
    pub fn hash_code(&self) -> i32 {
        // Never return s.hashCode(), so that
        // Key.hashCode() is the same whether we have cached s or not.
        if self.length == 0 {
            return 0;
        }

        let mut h = self.byte(0) as i32;
        for _ in 1..self.length {
            h = h.wrapping_mul(37).wrapping_add(self.byte(0) as i32);
        }
        h
    }

    /// `compareTo(CharSequence cs)`.
    pub fn compare_to_cs(&self, cs: &str) -> i32 {
        let cs: Vec<u16> = cs.encode_utf16().collect();
        let cs_length = cs.len() as i32;
        let min_length = self.length.min(cs_length);
        for i in 0..min_length {
            let diff = self.char_at(i) as i32 - cs[i as usize] as i32;
            if diff != 0 {
                return diff;
            }
        }
        self.length - cs_length
    }

    /// `compareTo(Key other)`.
    pub fn compare_to(&self, other: &Key) -> i32 {
        let min_length = self.length.min(other.length);
        for i in 0..min_length {
            let diff = self.char_at(i) as i32 - other.char_at(i) as i32;
            if diff != 0 {
                return diff;
            }
        }
        self.length - other.length
    }

    /// The key as UTF-16 units, the `CharSequence` the table lookups take.
    pub fn units(&self) -> Vec<u16> {
        (0..self.length).map(|i| self.char_at(i)).collect()
    }
}

/// `equals(Object)`: same length and bytes.
impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.length == other.length && self.region_matches_bytes(other, self.length)
    }
}

impl Eq for Key {}

impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare_to(other).cmp(&0)
    }
}

/// Interface for iterating over a resource bundle array resource.
///
/// Equivalent to `com.ibm.icu.impl.UResource.Array`.
pub trait Array: Send + Sync {
    /// `getSize()`.
    fn get_size(&self) -> i32;
    /// `getValue(int i, Value value)`: sets `value` to the `i`'th item;
    /// `false` when `i` is out of range.
    fn get_value(&self, i: i32, value: &mut dyn Value) -> bool;
}

/// Interface for iterating over a resource bundle table resource.
///
/// Equivalent to `com.ibm.icu.impl.UResource.Table`.
pub trait Table: Send + Sync {
    /// `getSize()`.
    fn get_size(&self) -> i32;
    /// `getKeyAndValue(int i, Key key, Value value)`.
    fn get_key_and_value(&self, i: i32, key: &mut Key, value: &mut dyn Value) -> bool;
    /// `findValue(CharSequence key, Value value)`.
    fn find_value(&self, key: &[u16], value: &mut dyn Value) -> bool;
}

/// Represents a resource bundle item's value.
///
/// Equivalent to the abstract class `com.ibm.icu.impl.UResource.Value`.
/// Its accessors throw `UResourceTypeMismatchException` when the value has
/// another type.
pub trait Value: Any {
    /// `getType()`: one of the `UResourceBundle` types.
    fn get_type(&self) -> i32;
    /// `getString()`.
    fn get_string(&self) -> Result<Vec<u16>>;
    /// `getAliasString()`.
    fn get_alias_string(&self) -> Result<Vec<u16>>;
    /// `getInt()`.
    fn get_int(&self) -> Result<i32>;
    /// `getUInt()`.
    fn get_u_int(&self) -> Result<i32>;
    /// `getIntVector()`.
    fn get_int_vector(&self) -> Result<Vec<i32>>;
    /// `getBinary()`.
    fn get_binary(&self) -> Result<ByteBuffer>;
    /// `getArray()`.
    fn get_array(&self) -> Result<Arc<dyn Array>>;
    /// `getTable()`.
    fn get_table(&self) -> Result<Arc<dyn Table>>;
    /// `isNoInheritanceMarker()`.
    fn is_no_inheritance_marker(&self) -> bool;
    /// `getStringArray()`.
    fn get_string_array(&self) -> Result<Vec<Vec<u16>>>;
    /// `getStringArrayOrStringAsArray()`.
    fn get_string_array_or_string_as_array(&self) -> Result<Vec<Vec<u16>>>;
    /// `getStringOrFirstOfArray()`.
    fn get_string_or_first_of_array(&self) -> Result<Vec<u16>>;
    /// The value as `Any`, for the casts of the reader's containers.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// `toString()`: only for debugging.
    fn to_java_string(&self) -> String {
        match self.get_type() {
            UResourceBundle::STRING => self
                .get_string()
                .map(|s| String::from_utf16_lossy(&s))
                .unwrap_or_default(),
            UResourceBundle::INT => self.get_int().map(|i| i.to_string()).unwrap_or_default(),
            UResourceBundle::INT_VECTOR => {
                let iv = self.get_int_vector().unwrap_or_default();
                let mut sb = format!("[{}]{{", iv.len());
                if !iv.is_empty() {
                    sb.push_str(&iv[0].to_string());
                    for v in &iv[1..] {
                        sb.push_str(", ");
                        sb.push_str(&v.to_string());
                    }
                }
                sb.push('}');
                sb
            }
            UResourceBundle::BINARY => "(binary blob)".to_string(),
            UResourceBundle::ARRAY => "(array)".to_string(),
            UResourceBundle::TABLE => "(table)".to_string(),
            _ => "???".to_string(), // should not occur
        }
    }
}

/// Sink for ICU resource bundle contents.
///
/// Equivalent to the abstract class `com.ibm.icu.impl.UResource.Sink`.
pub trait Sink {
    /// `put(Key key, Value value, boolean noFallback)`: called once for
    /// each bundle (child-parent-...-root); `noFallback` is set when the
    /// item has no fallback beyond this bundle.
    ///
    /// # Errors
    ///
    /// Whatever the sink throws (Java's unchecked exceptions).
    fn put(&mut self, key: &mut Key, value: &mut dyn Value, no_fallback: bool) -> Result<()>;
}
