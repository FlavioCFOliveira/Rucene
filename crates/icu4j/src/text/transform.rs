// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.Transform`.

/// A transform from a source object to a destination object.
///
/// Equivalent to the interface `com.ibm.icu.text.Transform<S,D>`.
pub trait Transform<S, D> {
    /// `D transform(S source)`.
    fn transform(&self, source: S) -> D;
}
