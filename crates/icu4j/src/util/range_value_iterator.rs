// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.RangeValueIterator`.

/// Return result wrapper for `RangeValueIterator`.
///
/// Equivalent to `RangeValueIterator.Element`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Element {
    /// `start`: starting integer of the continuous range.
    pub start: i32,
    /// `limit`: (end + 1) integer of the continuous range.
    pub limit: i32,
    /// `value`: value of the continuous range.
    pub value: i32,
}

/// Interface for enabling iteration over sets of `<int index, int value>`,
/// where index is the sorted integer index in ascending order and value,
/// its associated integer value.
///
/// Equivalent to the interface `com.ibm.icu.util.RangeValueIterator`.
pub trait RangeValueIterator {
    /// `next(Element element)`: the next range; false at the end.
    fn next(&mut self, element: &mut Element) -> bool;

    /// `reset()`.
    fn reset(&mut self);
}
