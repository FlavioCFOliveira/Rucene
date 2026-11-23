// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.RawCollationKey`.

use std::ops::{Deref, DerefMut};

use crate::util::byte_array_wrapper::ByteArrayWrapper;

/// Simple class wrapper to store the internal byte representation of a
/// CollationKey. Unlike the CollationKey, this class do not contain a
/// reference to the source string, and is not immutable.
///
/// Equivalent to `com.ibm.icu.text.RawCollationKey` (a subclass of
/// `ByteArrayWrapper`, which it dereferences to).
#[derive(Debug, Clone, Default)]
pub struct RawCollationKey(pub ByteArrayWrapper);

impl Deref for RawCollationKey {
    type Target = ByteArrayWrapper;

    fn deref(&self) -> &ByteArrayWrapper {
        &self.0
    }
}

impl DerefMut for RawCollationKey {
    fn deref_mut(&mut self) -> &mut ByteArrayWrapper {
        &mut self.0
    }
}

impl RawCollationKey {
    /// Default constructor, internal byte array will be null and its size
    /// set to 0.
    ///
    /// Equivalent to `RawCollationKey()`.
    pub fn new() -> Self {
        Self(ByteArrayWrapper::new())
    }

    /// RawCollationKey created with an empty internal byte array of length
    /// capacity. Size of the internal byte array will be set to 0.
    ///
    /// Equivalent to `RawCollationKey(int capacity)`.
    ///
    /// # Panics
    ///
    /// Java's `NegativeArraySizeException` for a negative capacity.
    pub fn with_capacity(capacity: i32) -> Self {
        if capacity < 0 {
            panic!("java.lang.NegativeArraySizeException: {capacity}");
        }
        let mut w = ByteArrayWrapper::new();
        w.bytes = Some(vec![0; capacity as usize]);
        Self(w)
    }

    /// RawCollationKey created, adopting bytes as the internal byte array.
    /// Size of the internal byte array will be set to 0.
    ///
    /// Equivalent to `RawCollationKey(byte[] bytes)`.
    pub fn with_bytes(bytes: Option<Vec<i8>>) -> Self {
        let mut w = ByteArrayWrapper::new();
        w.bytes = bytes;
        Self(w)
    }

    /// Construct a RawCollationKey from a byte array and size.
    ///
    /// Equivalent to `RawCollationKey(byte[] bytesToAdopt, int size)`.
    pub fn with_bytes_and_size(bytes_to_adopt: Option<Vec<i8>>, size: i32) -> Self {
        Self(ByteArrayWrapper::with_bytes(bytes_to_adopt, size))
    }

    /// Compare this RawCollationKey to another, which must not be null.
    /// This overrides the inherited compareTo in order to adhere to the
    /// contract of a collation key: -1, 0 or 1.
    ///
    /// Equivalent to `compareTo(RawCollationKey)`.
    pub fn compare_to(&self, rhs: &RawCollationKey) -> i32 {
        let result = self.0.compare_to(&rhs.0);
        if result < 0 {
            -1
        } else if result == 0 {
            0
        } else {
            1
        }
    }
}
