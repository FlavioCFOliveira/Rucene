// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.CollationKey`.

use std::sync::atomic::{AtomicI32, Ordering};

use rucene::error::{LuceneError, Result};

use crate::r#impl::coll::collation::Collation;
use crate::text::collator;
use crate::text::raw_collation_key::RawCollationKey;

/// Options that used in the API CollationKey.getBound() for getting a
/// CollationKey based on the bound mode requested.
///
/// Equivalent to `CollationKey.BoundMode`.
#[derive(Debug)]
pub struct BoundMode;

impl BoundMode {
    /// Lower bound.
    pub const LOWER: i32 = 0;
    /// Upper bound that will match strings of exact size.
    pub const UPPER: i32 = 1;
    /// Upper bound that will match all the strings that have the same
    /// initial substring as the given string.
    pub const UPPER_LONG: i32 = 2;
    /// One more than the highest normal BoundMode value.
    #[deprecated]
    pub const COUNT: i32 = 3;
}

const MERGE_SEPERATOR_: i8 = 2;

/// A CollationKey represents a String under the rules of a specific
/// Collator object.
///
/// Equivalent to `com.ibm.icu.text.CollationKey`. Java's lazily computed
/// `m_hashCode_` and `m_length_` are atomics, so that the getters stay
/// `&self` as in Java.
#[derive(Debug)]
pub struct CollationKey {
    m_key_: Vec<i8>,
    m_source_: Option<Vec<u16>>,
    m_hash_code_: AtomicI32,
    m_length_: AtomicI32,
}

impl Clone for CollationKey {
    fn clone(&self) -> Self {
        Self {
            m_key_: self.m_key_.clone(),
            m_source_: self.m_source_.clone(),
            m_hash_code_: AtomicI32::new(self.m_hash_code_.load(Ordering::Relaxed)),
            m_length_: AtomicI32::new(self.m_length_.load(Ordering::Relaxed)),
        }
    }
}

impl CollationKey {
    /// CollationKey constructor, which allows CollationKey to be created
    /// from a byte array (Collator.getCollationKey would be the preferred
    /// way).
    ///
    /// Equivalent to `CollationKey(String source, byte key[])`.
    pub fn new(source: Option<Vec<u16>>, key: Vec<i8>) -> Self {
        Self::with_length(source, key, -1)
    }

    fn with_length(source: Option<Vec<u16>>, key: Vec<i8>, length: i32) -> Self {
        Self { m_key_: key, m_source_: source, m_hash_code_: AtomicI32::new(0), m_length_: AtomicI32::new(length) }
    }

    /// Private Constructor: for internal use only.
    ///
    /// Equivalent to `CollationKey(String source, RawCollationKey key)`.
    ///
    /// # Panics
    ///
    /// Java's `NullPointerException` when the key has no bytes.
    pub fn from_raw(source: Option<Vec<u16>>, key: &mut RawCollationKey) -> Self {
        let length = key.size - 1;
        let m_key_ = key.release_bytes().unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        debug_assert!(m_key_[length as usize] == 0);
        Self::with_length(source, m_key_, length)
    }

    /// Return the source string that this CollationKey represents.
    ///
    /// Equivalent to `getSourceString()`.
    pub fn get_source_string(&self) -> Option<&[u16]> {
        self.m_source_.as_deref()
    }

    /// Duplicates and returns the value of this CollationKey as a sequence
    /// of big-endian bytes terminated by a null.
    ///
    /// Equivalent to `toByteArray()`.
    pub fn to_byte_array(&self) -> Vec<i8> {
        let length = self.get_length() + 1;
        self.m_key_[..length as usize].to_vec()
    }

    /// Compare this CollationKey to another CollationKey.
    ///
    /// Equivalent to `compareTo(CollationKey)`.
    pub fn compare_to(&self, target: &CollationKey) -> i32 {
        let mut i = 0;
        loop {
            let l = self.m_key_[i] as i32 & 0xff;
            let r = target.m_key_[i] as i32 & 0xff;
            if l < r {
                return -1;
            } else if l > r {
                return 1;
            } else if l == 0 {
                return 0;
            }
            i += 1;
        }
    }

    /// Compare this CollationKey and the specified CollationKey for equality.
    ///
    /// Equivalent to `equals(CollationKey)`; `None` for Java's `null`.
    pub fn equals(&self, target: Option<&CollationKey>) -> bool {
        let Some(other) = target else {
            return false;
        };
        if std::ptr::eq(self, other) {
            return true;
        }
        let mut i = 0;
        loop {
            if self.m_key_[i] != other.m_key_[i] {
                return false;
            }
            if self.m_key_[i] == 0 {
                break;
            }
            i += 1;
        }
        true
    }

    /// Returns a hash code for this CollationKey.
    ///
    /// Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        if self.m_hash_code_.load(Ordering::Relaxed) == 0 {
            // m_key_ is never null here (Java's null check sets 1).
            let mut key: Vec<u16> = Vec::with_capacity(self.m_key_.len() >> 1);
            let mut i = 0;
            while self.m_key_[i] != 0 && self.m_key_[i + 1] != 0 {
                key.push((((self.m_key_[i] as i32) << 8) | (0xff & self.m_key_[i + 1] as i32)) as u16);
                i += 2;
            }
            if self.m_key_[i] != 0 {
                key.push(((self.m_key_[i] as i32) << 8) as u16);
            }
            let mut h: i32 = 0;
            for u in key {
                h = h.wrapping_mul(31).wrapping_add(u as i32);
            }
            self.m_hash_code_.store(h, Ordering::Relaxed);
        }
        self.m_hash_code_.load(Ordering::Relaxed)
    }

    /// Produces a bound for the sort order of a given collation key and a
    /// strength level.
    ///
    /// Equivalent to `getBound(int, int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for too many levels or an illegal bound
    /// type.
    pub fn get_bound(&self, bound_type: i32, mut no_of_levels: i32) -> Result<CollationKey> {
        // Scan the string until we skip enough of the key OR reach the end of
        // the key
        let mut offset: i32 = 0;
        let mut keystrength = collator::PRIMARY;
        let key = &self.m_key_;
        let len = key.len() as i32;

        if no_of_levels > collator::PRIMARY {
            while offset < len && key[offset as usize] != 0 {
                let b = key[offset as usize];
                offset += 1;
                if b as i32 == Collation::LEVEL_SEPARATOR_BYTE {
                    keystrength += 1;
                    no_of_levels -= 1;
                    if no_of_levels == collator::PRIMARY || offset == len || key[offset as usize] == 0 {
                        offset -= 1;
                        break;
                    }
                }
            }
        }

        if no_of_levels > 0 {
            return Err(LuceneError::illegal_argument(format!(
                "Source collation key has only {keystrength} strength level. Call getBound() again  with noOfLevels < {keystrength}"
            )));
        }

        // READ ME: this code assumes that the values for BoundMode variables
        // will not change. They are set so that the enum value corresponds to
        // the number of extra bytes each bound type needs.
        let size = offset + bound_type + 1;
        if size < 0 {
            panic!("java.lang.NegativeArraySizeException: {size}");
        }
        let mut resultkey = vec![0i8; size as usize];
        resultkey[..offset as usize].copy_from_slice(&key[..offset as usize]);
        match bound_type {
            BoundMode::LOWER => {
                // = 0
                // Lower bound just gets terminated. No extra bytes
            }
            BoundMode::UPPER => {
                // = 1
                // Upper bound needs one extra byte
                resultkey[offset as usize] = 2;
                offset += 1;
            }
            BoundMode::UPPER_LONG => {
                // = 2
                // Upper long bound needs two extra bytes
                resultkey[offset as usize] = 0xFFu8 as i8;
                offset += 1;
                resultkey[offset as usize] = 0xFFu8 as i8;
                offset += 1;
            }
            _ => return Err(LuceneError::illegal_argument("Illegal boundType argument")),
        }
        resultkey[offset as usize] = 0;
        Ok(CollationKey::with_length(None, resultkey, offset))
    }

    /// Merges this CollationKey with another.
    ///
    /// Equivalent to `merge(CollationKey)`; `None` for Java's `null`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a null or empty source.
    pub fn merge(&self, source: Option<&CollationKey>) -> Result<CollationKey> {
        // check arguments
        let source = match source {
            Some(s) if s.get_length() != 0 => s,
            _ => {
                return Err(LuceneError::illegal_argument("CollationKey argument can not be null or of 0 length"));
            }
        };

        // 1 byte extra for the 02 separator at the end of the copy of this sort key,
        // and 1 more for the terminating 00.
        let mut result = vec![0i8; (self.get_length() + source.get_length() + 2) as usize];

        // merge the sort keys with the same number of levels
        let mut rindex = 0usize;
        let mut index = 0usize;
        let mut sourceindex = 0usize;
        let mk = &self.m_key_;
        let sk = &source.m_key_;
        loop {
            // copy level from src1 not including 00 or 01
            // unsigned issues
            while mk[index] < 0 || mk[index] >= MERGE_SEPERATOR_ {
                result[rindex] = mk[index];
                rindex += 1;
                index += 1;
            }

            // add a 02 merge separator
            result[rindex] = MERGE_SEPERATOR_;
            rindex += 1;

            // copy level from src2 not including 00 or 01
            while sk[sourceindex] < 0 || sk[sourceindex] >= MERGE_SEPERATOR_ {
                result[rindex] = sk[sourceindex];
                rindex += 1;
                sourceindex += 1;
            }

            // if both sort keys have another level, then add a 01 level
            // separator and continue
            if mk[index] as i32 == Collation::LEVEL_SEPARATOR_BYTE
                && sk[sourceindex] as i32 == Collation::LEVEL_SEPARATOR_BYTE
            {
                index += 1;
                sourceindex += 1;
                result[rindex] = Collation::LEVEL_SEPARATOR_BYTE as i8;
                rindex += 1;
            } else {
                break;
            }
        }

        // here, at least one sort key is finished now, but the other one
        // might have some contents left from containing more levels;
        // that contents is just appended to the result
        let mut remaining_length = self.m_length_.load(Ordering::Relaxed) - index as i32;
        if remaining_length > 0 {
            let r = remaining_length as usize;
            result[rindex..rindex + r].copy_from_slice(&mk[index..index + r]);
            rindex += r;
        } else {
            remaining_length = source.m_length_.load(Ordering::Relaxed) - sourceindex as i32;
            if remaining_length > 0 {
                let r = remaining_length as usize;
                result[rindex..rindex + r].copy_from_slice(&sk[sourceindex..sourceindex + r]);
                rindex += r;
            }
        }
        result[rindex] = 0;

        debug_assert!(rindex == result.len() - 1);
        Ok(CollationKey::with_length(None, result, rindex as i32))
    }

    /// Gets the length of this CollationKey.
    fn get_length(&self) -> i32 {
        let cached = self.m_length_.load(Ordering::Relaxed);
        if cached >= 0 {
            return cached;
        }
        let mut length = self.m_key_.len() as i32;
        for (index, &b) in self.m_key_.iter().enumerate() {
            if b == 0 {
                length = index as i32;
                break;
            }
        }
        self.m_length_.store(length, Ordering::Relaxed);
        length
    }
}

impl PartialEq for CollationKey {
    /// `equals(Object)`.
    fn eq(&self, other: &Self) -> bool {
        self.equals(Some(other))
    }
}
