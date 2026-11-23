// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.UResourceBundleIterator`.

use rucene::error::{LuceneError, Result};

use crate::util::u_resource_bundle::UResourceBundle;

/// Class for enabling iteration over `UResourceBundle` objects.
///
/// Equivalent to `com.ibm.icu.util.UResourceBundleIterator`.
#[derive(Debug, Clone)]
pub struct UResourceBundleIterator {
    bundle: UResourceBundle,
    index: i32,
    size: i32,
}

/// `new NoSuchElementException()`.
fn no_such_element() -> LuceneError {
    crate::jdk::unchecked_no_message("java.util.NoSuchElementException")
}

impl UResourceBundleIterator {
    /// `new UResourceBundleIterator(UResourceBundle bndl)`.
    pub fn new(bndl: UResourceBundle) -> Self {
        let size = bndl.get_size();
        Self {
            bundle: bndl,
            index: 0,
            size,
        }
    }

    /// `next()`: the next resource.
    ///
    /// # Errors
    ///
    /// `NoSuchElementException` past the end; the errors of
    /// `UResourceBundle.get(int)`.
    pub fn next(&mut self) -> Result<UResourceBundle> {
        if self.index < self.size {
            let i = self.index;
            self.index += 1;
            return self.bundle.get_at(i);
        }
        Err(no_such_element())
    }

    /// `nextString()`: the next resource as a string.
    ///
    /// # Errors
    ///
    /// `NoSuchElementException` past the end;
    /// `UResourceTypeMismatchException` when it is not a string.
    pub fn next_string(&mut self) -> Result<Vec<u16>> {
        if self.index < self.size {
            let i = self.index;
            self.index += 1;
            return self.bundle.get_string_at(i);
        }
        Err(no_such_element())
    }

    /// `reset()`: resets the internal context of the iterator.
    pub fn reset(&mut self) {
        //reset the internal context
        self.index = 0;
    }

    /// `hasNext()`.
    pub fn has_next(&self) -> bool {
        self.index < self.size
    }
}
