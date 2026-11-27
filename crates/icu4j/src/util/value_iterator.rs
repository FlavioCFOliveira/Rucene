// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.ValueIterator`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;

/// The return result container of each iteration.
///
/// Equivalent to `ValueIterator.Element` (`value` is Java's `Object`).
#[derive(Debug, Clone, Default)]
pub struct Element {
    /// `integer`: integer result of the iteration.
    pub integer: i32,
    /// `value`: the object result of the iteration.
    pub value: Option<Arc<dyn Any + Send + Sync>>,
}

impl Element {
    /// `Element()`.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Interface for enabling iteration over sets of `<int, Object>`, where int
/// is the sorted integer index in ascending order, and Object its
/// associated value.
///
/// Equivalent to the interface `com.ibm.icu.util.ValueIterator`.
pub trait ValueIterator {
    /// `next(Element element)`: the next element; false at the end.
    fn next(&mut self, element: &mut Element) -> bool;

    /// `reset()`.
    fn reset(&mut self);

    /// `setRange(int start, int limit)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid range.
    fn set_range(&mut self, start: i32, limit: i32) -> Result<()>;
}
