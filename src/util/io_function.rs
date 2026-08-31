//! Fallible functional interfaces and iterator helpers ported from
//! `org.apache.lucene.util`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`IOSupplier`] | `IOSupplier<T>` |
//! | [`IOBooleanSupplier`] | `IOBooleanSupplier` |
//! | [`IOConsumer`] | `IOConsumer<T>` |
//! | [`IOFunction`] | `IOFunction<T, R>` |
//! | [`FloatToFloatFunction`] | `FloatToFloatFunction` |
//! | [`FilterIterator`] | `FilterIterator<T, InnerT>` |
//!
//! Java declares each of these as a `@FunctionalInterface` whose single method
//! may throw `IOException`. The Rust equivalent of "may throw `IOException`" is
//! returning [`crate::Result`], and the Rust equivalent of a functional
//! interface is a bound on `Fn`/`FnMut`, so each becomes a trait alias-style
//! `trait` with a blanket implementation: a closure of the right shape *is* an
//! `IOSupplier`, with no wrapper type and no boxing at the call site.

#![deny(unsafe_code)]

use crate::error::Result;

/// A result supplier that is allowed to fail.
///
/// Port of `org.apache.lucene.util.IOSupplier`.
pub trait IOSupplier<T> {
    /// Gets the result.
    ///
    /// # Errors
    ///
    /// Returns whatever error producing the result raised, standing in for
    /// Java's `IOException`.
    fn get(&mut self) -> Result<T>;
}

impl<T, F> IOSupplier<T> for F
where
    F: FnMut() -> Result<T>,
{
    fn get(&mut self) -> Result<T> {
        self()
    }
}

/// A boolean supplier that is allowed to fail.
///
/// Port of `org.apache.lucene.util.IOBooleanSupplier`.
pub trait IOBooleanSupplier {
    /// Gets the boolean result.
    ///
    /// # Errors
    ///
    /// Returns whatever error producing the result raised.
    fn get(&mut self) -> Result<bool>;
}

impl<F> IOBooleanSupplier for F
where
    F: FnMut() -> Result<bool>,
{
    fn get(&mut self) -> Result<bool> {
        self()
    }
}

/// An operation over a single input that is allowed to fail.
///
/// Port of `org.apache.lucene.util.IOConsumer`.
pub trait IOConsumer<T> {
    /// Performs this operation on `input`.
    ///
    /// # Errors
    ///
    /// Returns whatever error the operation raised.
    fn accept(&mut self, input: T) -> Result<()>;
}

impl<T, F> IOConsumer<T> for F
where
    F: FnMut(T) -> Result<()>,
{
    fn accept(&mut self, input: T) -> Result<()> {
        self(input)
    }
}

/// A function that is allowed to fail.
///
/// Port of `org.apache.lucene.util.IOFunction`.
pub trait IOFunction<T, R> {
    /// Applies this function to `t`.
    ///
    /// # Errors
    ///
    /// Returns whatever error producing the result raised.
    fn apply(&mut self, t: T) -> Result<R>;
}

impl<T, R, F> IOFunction<T, R> for F
where
    F: FnMut(T) -> Result<R>,
{
    fn apply(&mut self, t: T) -> Result<R> {
        self(t)
    }
}

/// Maps one `f32` to another, useful when scaling scores.
///
/// Port of `org.apache.lucene.util.FloatToFloatFunction`.
pub trait FloatToFloatFunction {
    /// Applies this function to `f`.
    fn apply(&self, f: f32) -> f32;
}

impl<F> FloatToFloatFunction for F
where
    F: Fn(f32) -> f32,
{
    fn apply(&self, f: f32) -> f32 {
        self(f)
    }
}

/// An [`Iterator`] that keeps only the elements satisfying a predicate.
///
/// Port of `org.apache.lucene.util.FilterIterator`.
///
/// **Divergence from Lucene 10.5.0.** Java's class is abstract, with
/// `predicateFunction` supplied by the subclass, and its `remove()` throws
/// `UnsupportedOperationException`. Rust iterators have no `remove`, and a
/// closure is the natural way to supply the predicate, so this is a concrete
/// type parameterised by one.
pub struct FilterIterator<I, P>
where
    I: Iterator,
    P: FnMut(&I::Item) -> bool,
{
    iterator: I,
    predicate: P,
    next: Option<I::Item>,
    next_is_set: bool,
}

impl<I, P> FilterIterator<I, P>
where
    I: Iterator,
    P: FnMut(&I::Item) -> bool,
{
    /// Wraps `base_iterator`, keeping only elements for which `predicate`
    /// returns `true`.
    pub fn new(base_iterator: I, predicate: P) -> Self {
        Self {
            iterator: base_iterator,
            predicate,
            next: None,
            next_is_set: false,
        }
    }

    /// Returns whether another element is available.
    ///
    /// Equivalent to `FilterIterator.hasNext`.
    pub fn has_next(&mut self) -> bool {
        self.next_is_set || self.set_next()
    }

    /// Equivalent to the private `FilterIterator.setNext`.
    fn set_next(&mut self) -> bool {
        debug_assert!(!self.next_is_set);
        for object in self.iterator.by_ref() {
            if (self.predicate)(&object) {
                self.next = Some(object);
                self.next_is_set = true;
                return true;
            }
        }
        false
    }
}

impl<I, P> Iterator for FilterIterator<I, P>
where
    I: Iterator,
    P: FnMut(&I::Item) -> bool,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.has_next() {
            return None;
        }
        debug_assert!(self.next_is_set);
        self.next_is_set = false;
        self.next.take()
    }
}
