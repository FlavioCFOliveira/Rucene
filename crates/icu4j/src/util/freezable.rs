// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.Freezable`.

/// An object that can be frozen (made immutable) and cloned unfrozen.
///
/// Equivalent to the interface `com.ibm.icu.util.Freezable<T>`.
pub trait Freezable<T> {
    /// `isFrozen()`.
    fn is_frozen(&self) -> bool;

    /// `freeze()`: makes the object immutable and returns it.
    fn freeze(&mut self) -> &mut T;

    /// `cloneAsThawed()`: a mutable copy.
    fn clone_as_thawed(&self) -> T;
}
